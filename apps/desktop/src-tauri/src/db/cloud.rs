//! Persistencia de la sincronización con la nube (F11): la cola de salida
//! (`sync_outbox`) y las lecturas/escrituras locales que necesita `sync::engine`. Sin
//! tipos de `sync`: igual que `queries.rs` no depende de `catalog`, aquí entran y salen
//! datos planos, y la lógica de qué gana en un conflicto vive en `sync`.

use rusqlite::{params, Connection, OptionalExtension};

use super::reads::PlaybackItem;
use super::{now_epoch, Result};

// ─────────────── Cola de salida ───────────────

pub struct OutboxEntry {
    pub id: i64,
    pub target: String,
    pub payload: String,
    pub revision: i64,
    pub attempts: i64,
}

/// Apunta (o sustituye, si ya había uno pendiente para el mismo `target`) un cambio que
/// tiene que llegar a la nube. `not_before` permite agrupar escrituras frecuentes: al
/// sustituir se queda el más temprano de los dos, así un cambio que ya tocaba enviar no
/// se retrasa por otro que llegue después.
pub fn enqueue(conn: &Connection, target: &str, collection: &str, payload: &str, not_before: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_outbox (target, collection, payload, next_attempt_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(target) DO UPDATE SET
            collection = excluded.collection,
            payload = excluded.payload,
            revision = sync_outbox.revision + 1,
            attempts = 0,
            last_error = NULL,
            next_attempt_at = MIN(sync_outbox.next_attempt_at, excluded.next_attempt_at)",
        params![target, collection, payload, not_before, now_epoch()],
    )?;
    Ok(())
}

/// Entradas a las que ya les toca intentarse, en el orden en que se apuntaron.
pub fn due_entries(conn: &Connection, now: i64, limit: i64) -> Result<Vec<OutboxEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, target, payload, revision, attempts FROM sync_outbox
         WHERE next_attempt_at <= ?1 ORDER BY id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![now, limit], |row| {
        Ok(OutboxEntry { id: row.get(0)?, target: row.get(1)?, payload: row.get(2)?, revision: row.get(3)?, attempts: row.get(4)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Da por enviada una entrada, solo si nadie la sustituyó mientras estaba en vuelo.
pub fn ack(conn: &Connection, id: i64, revision: i64) -> Result<()> {
    conn.execute("DELETE FROM sync_outbox WHERE id = ?1 AND revision = ?2", params![id, revision])?;
    Ok(())
}

/// Apunta un intento fallido y cuándo volver a probar.
pub fn record_failure(conn: &Connection, id: i64, revision: i64, error: &str, next_attempt_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE sync_outbox SET attempts = attempts + 1, last_error = ?1, next_attempt_at = ?2
         WHERE id = ?3 AND revision = ?4",
        params![error, next_attempt_at, id, revision],
    )?;
    Ok(())
}

pub fn pending_count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0)).map_err(Into::into)
}

/// Un cambio local todavía sin enviar sobre este documento gana a lo que diga la nube:
/// se enviará igualmente y la sobrescribirá.
pub fn has_pending_target(conn: &Connection, target: &str) -> Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE target = ?1)", params![target], |row| row.get(0)).map_err(Into::into)
}

pub fn clear_outbox(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM sync_outbox", [])?;
    Ok(())
}

/// Al borrar un perfil, sus favoritos y progreso pendientes ya no tienen sentido.
/// `substr` en vez de `LIKE` para que ningún carácter del id actúe de comodín.
pub fn drop_pending_for_profile(conn: &Connection, remote_profile_id: &str) -> Result<()> {
    for prefix in [format!("favorite:{remote_profile_id}:"), format!("playback:{remote_profile_id}:")] {
        conn.execute(
            "DELETE FROM sync_outbox WHERE substr(target, 1, length(?1)) = ?1",
            params![prefix],
        )?;
    }
    Ok(())
}

/// Borra los ajustes que empiezan por `prefix` (cursores de bajada de una cuenta que
/// cierra sesión, ver `sync::engine`).
pub fn delete_settings_with_prefix(conn: &Connection, prefix: &str) -> Result<()> {
    conn.execute("DELETE FROM app_settings WHERE substr(key, 1, length(?1)) = ?1", params![prefix])?;
    Ok(())
}

// ─────────────── Perfiles ───────────────

pub struct SyncProfile {
    pub id: i64,
    pub remote_id: Option<String>,
    pub name: String,
    pub avatar: Option<String>,
    pub adult_ok: bool,
    pub pin_hash: Option<String>,
    pub updated_at: i64,
}

const PROFILE_COLUMNS: &str = "id, remote_id, name, avatar, adult_ok, pin_hash, updated_at";

fn row_to_sync_profile(row: &rusqlite::Row) -> rusqlite::Result<SyncProfile> {
    Ok(SyncProfile {
        id: row.get(0)?,
        remote_id: row.get(1)?,
        name: row.get(2)?,
        avatar: row.get(3)?,
        adult_ok: row.get::<_, i64>(4)? != 0,
        pin_hash: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub fn sync_profile(conn: &Connection, profile_id: i64) -> Result<Option<SyncProfile>> {
    conn.query_row(&format!("SELECT {PROFILE_COLUMNS} FROM profiles WHERE id = ?1"), params![profile_id], row_to_sync_profile)
        .optional()
        .map_err(Into::into)
}

pub fn sync_profiles(conn: &Connection) -> Result<Vec<SyncProfile>> {
    let mut stmt = conn.prepare(&format!("SELECT {PROFILE_COLUMNS} FROM profiles ORDER BY id"))?;
    let rows = stmt.query_map([], row_to_sync_profile)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn profile_by_remote_id(conn: &Connection, remote_id: &str) -> Result<Option<SyncProfile>> {
    conn.query_row(&format!("SELECT {PROFILE_COLUMNS} FROM profiles WHERE remote_id = ?1"), params![remote_id], row_to_sync_profile)
        .optional()
        .map_err(Into::into)
}

/// Crea o actualiza el perfil local espejo de uno remoto. `updated_at` es el de la nube,
/// no el de ahora: si no, el perfil recién bajado parecería un cambio local más nuevo y
/// se volvería a subir sin motivo.
pub fn upsert_profile_from_remote(
    conn: &Connection,
    remote_id: &str,
    name: &str,
    avatar: Option<&str>,
    adult_ok: bool,
    pin_hash: Option<&str>,
    updated_at: i64,
) -> Result<i64> {
    if let Some(existing) = profile_by_remote_id(conn, remote_id)? {
        conn.execute(
            "UPDATE profiles SET name = ?1, avatar = ?2, adult_ok = ?3, pin_hash = ?4, updated_at = ?5 WHERE id = ?6",
            params![name, avatar, adult_ok as i64, pin_hash, updated_at, existing.id],
        )?;
        return Ok(existing.id);
    }
    conn.execute(
        "INSERT INTO profiles (name, avatar, adult_ok, pin_hash, remote_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![name, avatar, adult_ok as i64, pin_hash, remote_id, now_epoch(), updated_at],
    )?;
    Ok(conn.last_insert_rowid())
}

// ─────────────── Favoritos ───────────────

pub struct SyncFavorite {
    pub item_kind: String,
    pub item_id: i64,
}

/// Favoritos sincronizables de un perfil: los de canal se quedan fuera, no tienen clave
/// portable entre proveedores (ver `reads::item_key_for`).
pub fn sync_favorites(conn: &Connection, profile_id: i64) -> Result<Vec<SyncFavorite>> {
    let mut stmt = conn.prepare("SELECT item_kind, item_id FROM favorites WHERE profile_id = ?1 AND item_kind <> 'channel'")?;
    let rows = stmt.query_map(params![profile_id], |row| Ok(SyncFavorite { item_kind: row.get(0)?, item_id: row.get(1)? }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn favorite_added_at(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT added_at FROM favorites WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
        params![profile_id, item_kind, item_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Igual que `reads::add_favorite`, pero conservando la fecha original de la nube.
pub fn put_favorite_from_remote(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64, added_at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO favorites (profile_id, item_kind, item_id, added_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(profile_id, item_kind, item_id) DO UPDATE SET added_at = excluded.added_at",
        params![profile_id, item_kind, item_id, added_at],
    )?;
    Ok(())
}

// ─────────────── Progreso de reproducción ───────────────

/// Progreso sincronizable de un perfil (sin canales: el directo no se "continúa").
pub fn sync_playback(conn: &Connection, profile_id: i64) -> Result<Vec<PlaybackItem>> {
    let mut stmt = conn.prepare(
        "SELECT item_kind, item_id, position_s, duration_s, finished, played_at FROM playback_state
         WHERE profile_id = ?1 AND item_kind <> 'channel'",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| {
        Ok(PlaybackItem {
            item_kind: row.get(0)?,
            item_id: row.get(1)?,
            position_s: row.get(2)?,
            duration_s: row.get(3)?,
            finished: row.get::<_, i64>(4)? != 0,
            played_at: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Igual que `reads::upsert_playback_progress`, pero conservando `played_at` y
/// `finished` tal como vienen de la nube: si se pusiera la hora de ahora, algo visto
/// hace semanas en otro dispositivo saltaría al principio de "Continuar viendo".
#[allow(clippy::too_many_arguments)]
pub fn put_playback_from_remote(
    conn: &Connection,
    profile_id: i64,
    item_kind: &str,
    item_id: i64,
    position_s: i64,
    duration_s: Option<i64>,
    finished: bool,
    played_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, duration_s, finished, played_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(profile_id, item_kind, item_id) DO UPDATE SET
            position_s = excluded.position_s, duration_s = excluded.duration_s,
            finished = excluded.finished, played_at = excluded.played_at",
        params![profile_id, item_kind, item_id, position_s, duration_s, finished as i64, played_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_connection_at;

    fn temp_db() -> Connection {
        use rand::RngExt;
        let suffix: u64 = rand::rng().random();
        let path = std::env::temp_dir().join(format!("vibe-iptv-cloud-test-{}-{suffix:x}.sqlite3", std::process::id()));
        open_connection_at(&path).unwrap()
    }

    #[test]
    fn a_newer_change_replaces_the_pending_one_and_bumps_its_revision() {
        let conn = temp_db();
        enqueue(&conn, "playback:p:movie:k", "playback", "{\"v\":1}", 100).unwrap();
        enqueue(&conn, "playback:p:movie:k", "playback", "{\"v\":2}", 160).unwrap();

        let due = due_entries(&conn, 1_000, 10).unwrap();
        assert_eq!(due.len(), 1, "un solo documento remoto, una sola entrada pendiente");
        assert_eq!(due[0].payload, "{\"v\":2}");
        assert_eq!(due[0].revision, 2);
    }

    #[test]
    fn replacing_keeps_the_earliest_send_time() {
        let conn = temp_db();
        enqueue(&conn, "favorite:p:movie:k", "favorites", "{}", 100).unwrap();
        enqueue(&conn, "favorite:p:movie:k", "favorites", "{}", 500).unwrap();
        assert_eq!(due_entries(&conn, 100, 10).unwrap().len(), 1);
    }

    #[test]
    fn ack_does_not_drop_a_change_that_arrived_while_sending() {
        let conn = temp_db();
        enqueue(&conn, "profile:abc", "profiles", "{\"v\":1}", 0).unwrap();
        let sent = due_entries(&conn, 0, 10).unwrap().remove(0);
        enqueue(&conn, "profile:abc", "profiles", "{\"v\":2}", 0).unwrap();

        ack(&conn, sent.id, sent.revision).unwrap();
        let still_pending = due_entries(&conn, 0, 10).unwrap();
        assert_eq!(still_pending.len(), 1);
        assert_eq!(still_pending[0].payload, "{\"v\":2}");

        ack(&conn, still_pending[0].id, still_pending[0].revision).unwrap();
        assert_eq!(pending_count(&conn).unwrap(), 0);
    }

    #[test]
    fn a_failure_postpones_the_entry() {
        let conn = temp_db();
        enqueue(&conn, "profile:abc", "profiles", "{}", 0).unwrap();
        let entry = due_entries(&conn, 0, 10).unwrap().remove(0);
        record_failure(&conn, entry.id, entry.revision, "sin red", 300).unwrap();

        assert!(due_entries(&conn, 299, 10).unwrap().is_empty());
        let retried = due_entries(&conn, 300, 10).unwrap();
        assert_eq!(retried[0].attempts, 1);
    }

    #[test]
    fn dropping_a_profile_only_removes_its_own_pending_items() {
        let conn = temp_db();
        enqueue(&conn, "favorite:p1:movie:k", "favorites", "{}", 0).unwrap();
        enqueue(&conn, "playback:p1:episode:k:1:2", "playback", "{}", 0).unwrap();
        enqueue(&conn, "favorite:p10:movie:k", "favorites", "{}", 0).unwrap();
        enqueue(&conn, "profile:p1", "profiles", "{}", 0).unwrap();

        drop_pending_for_profile(&conn, "p1").unwrap();
        let left: Vec<String> = due_entries(&conn, 0, 10).unwrap().into_iter().map(|e| e.target).collect();
        assert_eq!(left, vec!["favorite:p10:movie:k".to_string(), "profile:p1".to_string()]);
    }

    #[test]
    fn a_profile_pulled_from_the_cloud_keeps_the_cloud_timestamp() {
        let conn = temp_db();
        let id = upsert_profile_from_remote(&conn, "r1", "Ana", None, false, None, 1_234).unwrap();
        let profile = sync_profile(&conn, id).unwrap().unwrap();
        assert_eq!(profile.updated_at, 1_234);
        assert_eq!(profile.remote_id.as_deref(), Some("r1"));

        let same_id = upsert_profile_from_remote(&conn, "r1", "Ana María", Some("2"), true, None, 2_000).unwrap();
        assert_eq!(same_id, id);
        let updated = sync_profile(&conn, id).unwrap().unwrap();
        assert_eq!(updated.name, "Ana María");
        assert!(updated.adult_ok);
    }
}
