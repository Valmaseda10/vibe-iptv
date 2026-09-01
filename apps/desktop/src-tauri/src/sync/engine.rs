//! Motor de sincronización: empuja cambios locales a Supabase en segundo plano
//! (best-effort — nunca bloquea ni hace fallar la petición que los originó, igual que
//! `server::stream::record_tier`) y baja+fusiona lo que haya en la nube al iniciar
//! sesión. Ver `docs/DECISIONS.md` para el porqué y los límites exactos: perfiles,
//! favoritos y progreso sí; catálogo, credenciales de proveedor y streaming, nunca.
//!
//! Todo lo de aquí es opcional: sin sesión de Supabase (`session::get_valid` da `None`),
//! cada función no hace nada — la app funciona igual de bien sin cuenta.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::db::{queries, reads, schema};

use super::session;
use super::supabase::{Session, SupabaseClient};

/// `timestamptz` de Postgres en UTC, sin tirar de una librería de fechas nueva — ya
/// existe `util::civil_time` para exactamente esta conversión (EPG, catch-up).
fn epoch_to_iso8601(epoch: i64) -> String {
    let (y, m, d, h, mi, s) = crate::util::civil_time::epoch_to_ymd_hms(epoch);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_connection_at(&db_path).ok()?;
        f(&conn).ok()
    })
    .await
    .ok()
    .flatten()
}

// ─────────────── Perfiles ───────────────

struct LocalProfileForPush {
    name: String,
    avatar: Option<String>,
    adult_ok: bool,
    pin_hash: Option<String>,
    remote_id: Option<String>,
}

fn read_profile_for_push(conn: &rusqlite::Connection, profile_id: i64) -> crate::db::Result<Option<LocalProfileForPush>> {
    conn.query_row(
        "SELECT name, avatar, adult_ok, pin_hash, remote_id FROM profiles WHERE id = ?1",
        rusqlite::params![profile_id],
        |row| {
            Ok(LocalProfileForPush {
                name: row.get(0)?,
                avatar: row.get(1)?,
                adult_ok: row.get::<_, i64>(2)? != 0,
                pin_hash: row.get(3)?,
                remote_id: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Sube (crea o actualiza) un perfil local. Sin sesión, no hace nada. Si el perfil
/// nunca se había sincronizado, lo crea en Supabase y guarda el UUID resultante en
/// `profiles.remote_id` para las próximas veces.
pub async fn push_profile_upsert(db_path: PathBuf, local_profile_id: i64) {
    let Some(session) = session::get_valid().await else { return };
    let Some(profile) = with_db(db_path.clone(), move |conn| read_profile_for_push(conn, local_profile_id)).await.flatten() else {
        return;
    };

    let client = SupabaseClient::new();
    let now = epoch_to_iso8601(crate::db::now_epoch());

    match &profile.remote_id {
        Some(remote_id) => {
            let body = json!({ "name": profile.name, "avatar": profile.avatar, "adult_ok": profile.adult_ok, "pin_hash": profile.pin_hash, "updated_at": now });
            let filter = format!("id=eq.{remote_id}");
            if let Err(err) = client.patch(&session, "profiles", &filter, &body).await {
                tracing::warn!(error = %err, "no se pudo subir el cambio de perfil a Supabase");
            }
        }
        None => {
            let body = json!({
                "user_id": session.user_id,
                "name": profile.name,
                "avatar": profile.avatar,
                "adult_ok": profile.adult_ok,
                "pin_hash": profile.pin_hash,
                "updated_at": now,
            });
            match client.upsert(&session, "profiles", "", &body).await {
                Ok(rows) => {
                    if let Some(remote_id) = rows.first().and_then(|r| r.get("id")).and_then(Value::as_str) {
                        let remote_id = remote_id.to_string();
                        with_db(db_path, move |conn| queries::set_profile_remote_id(conn, local_profile_id, &remote_id)).await;
                    }
                }
                Err(err) => tracing::warn!(error = %err, "no se pudo crear el perfil en Supabase"),
            }
        }
    }
}

/// Borra el espejo remoto de un perfil ya borrado localmente. `remote_id` viene del
/// llamador porque, para cuando esto se ejecuta, la fila local ya no existe.
pub async fn push_profile_delete(remote_id: String) {
    let Some(session) = session::get_valid().await else { return };
    let client = SupabaseClient::new();
    let filter = format!("id=eq.{remote_id}");
    if let Err(err) = client.delete_rows(&session, "profiles", &filter).await {
        tracing::warn!(error = %err, "no se pudo borrar el perfil en Supabase");
    }
}

// ─────────────── Favoritos ───────────────

/// `None` si el perfil no está sincronizado todavía (sin sesión, o no le ha tocado su
/// primer `push_profile_upsert`) — sin un `remote_id` de perfil no hay bajo qué colgar
/// el favorito en Supabase.
async fn remote_profile_id(db_path: &Path, local_profile_id: i64) -> Option<String> {
    with_db(db_path.to_path_buf(), move |conn| {
        conn.query_row("SELECT remote_id FROM profiles WHERE id = ?1", rusqlite::params![local_profile_id], |r| r.get::<_, Option<String>>(0))
            .map_err(Into::into)
    })
    .await
    .flatten()
}

/// Canales fuera a propósito: sin `item_key` portable entre proveedores distintos (ver
/// `db::reads`, comentario de la sección "Sincronización"), así que un favorito de
/// directo se queda solo en este PC.
pub async fn push_favorite_add(db_path: PathBuf, local_profile_id: i64, item_kind: String, local_item_id: i64) {
    if item_kind == "channel" {
        return;
    }
    let Some(session) = session::get_valid().await else { return };
    let Some(remote_profile_id) = remote_profile_id(&db_path, local_profile_id).await else { return };
    let Some(item_key) = with_db(db_path, {
        let item_kind = item_kind.clone();
        move |conn| reads::item_key_for(conn, &item_kind, local_item_id)
    })
    .await
    .flatten() else {
        return;
    };

    let client = SupabaseClient::new();
    let body = json!({ "profile_id": remote_profile_id, "item_kind": item_kind, "item_key": item_key, "added_at": epoch_to_iso8601(crate::db::now_epoch()) });
    if let Err(err) = client.upsert(&session, "favorites", "on_conflict=profile_id,item_kind,item_key", &body).await {
        tracing::warn!(error = %err, "no se pudo subir el favorito a Supabase");
    }
}

/// El `item_key` debe resolverse ANTES de borrar la fila local (`server::api::delete_favorite`
/// lo hace así): una vez borrada, ya no habría de dónde sacar el `match_key`.
pub async fn push_favorite_remove_by_key(db_path: PathBuf, local_profile_id: i64, item_kind: String, item_key: String) {
    if item_kind == "channel" {
        return;
    }
    let Some(session) = session::get_valid().await else { return };
    let Some(remote_profile_id) = remote_profile_id(&db_path, local_profile_id).await else { return };
    let client = SupabaseClient::new();
    let filter = format!("profile_id=eq.{remote_profile_id}&item_kind=eq.{item_kind}&item_key=eq.{item_key}");
    if let Err(err) = client.delete_rows(&session, "favorites", &filter).await {
        tracing::warn!(error = %err, "no se pudo borrar el favorito en Supabase");
    }
}

// ─────────────── Progreso de reproducción ───────────────

pub async fn push_playback_progress(db_path: PathBuf, local_profile_id: i64, item_kind: String, local_item_id: i64, position_s: i64, duration_s: Option<i64>) {
    if item_kind == "channel" {
        return; // el directo no tiene "continuar viendo" real, ver db::queries.
    }
    let Some(session) = session::get_valid().await else { return };
    let Some(remote_profile_id) = remote_profile_id(&db_path, local_profile_id).await else { return };
    let Some(item_key) = with_db(db_path, {
        let item_kind = item_kind.clone();
        move |conn| reads::item_key_for(conn, &item_kind, local_item_id)
    })
    .await
    .flatten() else {
        return;
    };

    let finished = duration_s.is_some_and(|d| d > 0 && (position_s as f64 / d as f64) >= 0.9);
    let client = SupabaseClient::new();
    let body = json!({
        "profile_id": remote_profile_id,
        "item_kind": item_kind,
        "item_key": item_key,
        "position_s": position_s,
        "duration_s": duration_s,
        "finished": finished,
        "played_at": epoch_to_iso8601(crate::db::now_epoch()),
    });
    if let Err(err) = client.upsert(&session, "playback_state", "on_conflict=profile_id,item_kind,item_key", &body).await {
        tracing::warn!(error = %err, "no se pudo subir el progreso a Supabase");
    }
}

/// Igual que `push_favorite_remove_by_key`: el `item_key` hace falta ANTES de borrar la
/// fila local (`server::api::delete_playback_progress` lo hace así). "Quitar de
/// continuar viendo" (F "premium") debe quitarse también del resto de dispositivos, no
/// solo de este.
pub async fn push_playback_remove_by_key(db_path: PathBuf, local_profile_id: i64, item_kind: String, item_key: String) {
    if item_kind == "channel" {
        return;
    }
    let Some(session) = session::get_valid().await else { return };
    let Some(remote_profile_id) = remote_profile_id(&db_path, local_profile_id).await else { return };
    let client = SupabaseClient::new();
    let filter = format!("profile_id=eq.{remote_profile_id}&item_kind=eq.{item_kind}&item_key=eq.{item_key}");
    if let Err(err) = client.delete_rows(&session, "playback_state", &filter).await {
        tracing::warn!(error = %err, "no se pudo borrar el progreso en Supabase");
    }
}

// ─────────────── Bajada + fusión al iniciar sesión ───────────────

/// Se llama justo después de un login/registro con éxito. Recorre los perfiles del
/// usuario en Supabase, los fusiona con los locales (por `remote_id`; última escritura
/// gana comparando `updated_at`), sube los perfiles locales que todavía no tuvieran
/// pareja remota, y por cada perfil ya emparejado fusiona sus favoritos y su progreso.
///
/// Todo esto es best-effort: un fallo a media fusión no deja nada corrupto (cada
/// `INSERT`/`UPDATE` local es una operación pequeña e idempotente), solo dejaría el
/// resto por sincronizar hasta el próximo intento.
pub async fn pull_and_merge(db_path: PathBuf) {
    let Some(session) = session::get_valid().await else { return };
    let client = SupabaseClient::new();

    let remote_profiles = match client.select(&session, "profiles", "select=*").await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "no se pudo bajar la lista de perfiles de Supabase");
            return;
        }
    };

    for remote in &remote_profiles {
        merge_one_profile(&db_path, &session, &client, remote).await;
    }

    // Perfiles locales que nunca se habían sincronizado (creados sin sesión, o antes de
    // que existiera la cuenta): se suben ahora que hay con quién.
    let unsynced = with_db(db_path.clone(), |conn| {
        let mut stmt = conn.prepare("SELECT id FROM profiles WHERE remote_id IS NULL")?;
        let ids = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    })
    .await
    .unwrap_or_default();
    for local_id in unsynced {
        push_profile_upsert(db_path.clone(), local_id).await;
    }
}

fn get_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn get_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `"2026-08-31T12:00:00+00:00"` -> epoch. Solo hace falta comparar "¿cuál es más
/// reciente?", así que basta un parseo simple de los primeros campos numéricos — no se
/// necesita una librería de fechas para esto.
fn iso8601_to_epoch(value: &str) -> Option<i64> {
    let digits: Vec<i64> = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    let [y, mo, d, h, mi, s, ..] = digits[..] else { return None };
    let days = crate::util::civil_time::days_from_civil(y as i32, mo as u32, d as u32)?;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

async fn merge_one_profile(db_path: &Path, session: &Session, client: &SupabaseClient, remote: &Value) {
    let Some(remote_id) = get_str(remote, "id") else { return };
    let name = get_str(remote, "name").unwrap_or_else(|| "Perfil".to_string());
    let avatar = get_str(remote, "avatar");
    let adult_ok = get_bool(remote, "adult_ok");
    let pin_hash = get_str(remote, "pin_hash");
    let remote_updated = remote.get("updated_at").and_then(Value::as_str).and_then(iso8601_to_epoch).unwrap_or(0);

    let db_path_owned = db_path.to_path_buf();
    let local_match = with_db(db_path_owned.clone(), {
        let remote_id = remote_id.clone();
        move |conn| {
            conn.query_row("SELECT id, updated_at FROM profiles WHERE remote_id = ?1", rusqlite::params![remote_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()
            .map_err(Into::into)
        }
    })
    .await
    .flatten();

    let local_profile_id = match local_match {
        Some((local_id, local_updated)) => {
            if remote_updated > local_updated {
                let (name, avatar, pin_hash) = (name.clone(), avatar.clone(), pin_hash.clone());
                with_db(db_path_owned.clone(), move |conn| {
                    queries::update_profile(conn, local_id, &name, avatar.as_deref(), adult_ok)?;
                    queries::set_profile_pin_hash(conn, local_id, pin_hash.as_deref())
                })
                .await;
            }
            local_id
        }
        None => {
            // No existía en este PC todavía: viene de otro dispositivo.
            let (name2, avatar2, pin_hash2, remote_id2) = (name.clone(), avatar.clone(), pin_hash.clone(), remote_id.clone());
            let Some(new_id) = with_db(db_path_owned.clone(), move |conn| {
                let id = queries::insert_profile(conn, &name2, avatar2.as_deref(), adult_ok)?;
                queries::set_profile_pin_hash(conn, id, pin_hash2.as_deref())?;
                queries::set_profile_remote_id(conn, id, &remote_id2)?;
                Ok(id)
            })
            .await
            else {
                return;
            };
            new_id
        }
    };

    merge_favorites(db_path, session, client, &remote_id, local_profile_id).await;
    merge_playback_state(db_path, session, client, &remote_id, local_profile_id).await;
}

async fn merge_favorites(db_path: &Path, session: &Session, client: &SupabaseClient, remote_profile_id: &str, local_profile_id: i64) {
    let query = format!("select=*&profile_id=eq.{remote_profile_id}");
    let remote_favorites = match client.select(session, "favorites", &query).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "no se pudieron bajar los favoritos de este perfil");
            return;
        }
    };

    for fav in &remote_favorites {
        let (Some(item_kind), Some(item_key)) = (get_str(fav, "item_kind"), get_str(fav, "item_key")) else { continue };
        let db_path_owned = db_path.to_path_buf();
        let local_item_id = with_db(db_path_owned.clone(), {
            let item_kind = item_kind.clone();
            let item_key = item_key.clone();
            move |conn| reads::item_id_for_key(conn, &item_kind, &item_key)
        })
        .await
        .flatten();
        // Si el contenido no existe en el catálogo de este PC (proveedor distinto), se
        // deja tal cual: sigue en Supabase, se resolverá el día que este PC sí lo tenga.
        if let Some(local_item_id) = local_item_id {
            with_db(db_path_owned, move |conn| reads::add_favorite(conn, local_profile_id, &item_kind, local_item_id)).await;
        }
    }

    // Favoritos locales que Supabase todavía no conoce (se marcaron sin conexión, o
    // antes del primer login): se suben ahora.
    let local_favorites = with_db(db_path.to_path_buf(), move |conn| reads::list_favorites(conn, local_profile_id)).await.unwrap_or_default();
    for fav in local_favorites {
        push_favorite_add(db_path.to_path_buf(), local_profile_id, fav.item_kind, fav.item_id).await;
    }
}

async fn merge_playback_state(db_path: &Path, session: &Session, client: &SupabaseClient, remote_profile_id: &str, local_profile_id: i64) {
    let query = format!("select=*&profile_id=eq.{remote_profile_id}");
    let remote_rows = match client.select(session, "playback_state", &query).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "no se pudo bajar el progreso de este perfil");
            return;
        }
    };

    for row in &remote_rows {
        let (Some(item_kind), Some(item_key)) = (get_str(row, "item_kind"), get_str(row, "item_key")) else { continue };
        let position_s = row.get("position_s").and_then(Value::as_i64).unwrap_or(0);
        let duration_s = row.get("duration_s").and_then(Value::as_i64);
        let remote_played_at = row.get("played_at").and_then(Value::as_str).and_then(iso8601_to_epoch).unwrap_or(0);

        let db_path_owned = db_path.to_path_buf();
        let local_item_id = with_db(db_path_owned.clone(), {
            let item_kind = item_kind.clone();
            let item_key = item_key.clone();
            move |conn| reads::item_id_for_key(conn, &item_kind, &item_key)
        })
        .await
        .flatten();
        let Some(local_item_id) = local_item_id else { continue };

        let local_played_at = with_db(db_path_owned.clone(), {
            let item_kind = item_kind.clone();
            move |conn| {
                conn.query_row(
                    "SELECT played_at FROM playback_state WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
                    rusqlite::params![local_profile_id, item_kind, local_item_id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(Into::into)
            }
        })
        .await
        .flatten()
        .unwrap_or(0);

        // Última escritura gana: si lo de Supabase es más reciente que lo local, se
        // aplica aquí; si no, ya se subirá lo local por su cuenta (o en el bucle de
        // abajo, si nunca se había sincronizado este ítem).
        if remote_played_at > local_played_at {
            with_db(db_path_owned, move |conn| reads::upsert_playback_progress(conn, local_profile_id, &item_kind, local_item_id, position_s as i64, duration_s)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_to_iso8601_matches_a_known_instant() {
        // 1_700_000_000 = 2023-11-14T22:13:20Z (mismo instante usado en otros tests del
        // proyecto para esta conversión, ver sources::xtream).
        assert_eq!(epoch_to_iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn iso8601_to_epoch_roundtrips_epoch_to_iso8601() {
        let epoch = 1_700_000_000;
        assert_eq!(iso8601_to_epoch(&epoch_to_iso8601(epoch)), Some(epoch));
    }

    #[test]
    fn iso8601_to_epoch_ignores_fractional_seconds_and_timezone_suffix() {
        // Forma real que devuelve Postgres para un `timestamptz`: microsegundos y
        // desplazamiento horario, ninguno de los dos debe desviar el resultado — los
        // primeros seis grupos de dígitos son siempre año/mes/día/hora/min/seg.
        let epoch = iso8601_to_epoch("2023-11-14T22:13:20.123456+00:00").unwrap();
        assert_eq!(epoch, 1_700_000_000);
    }

    #[test]
    fn iso8601_to_epoch_rejects_garbage() {
        assert_eq!(iso8601_to_epoch("no es una fecha"), None);
        assert_eq!(iso8601_to_epoch(""), None);
    }

    #[test]
    fn get_str_and_get_bool_read_json_fields_defensively() {
        let value = serde_json::json!({ "name": "Jorge", "adult_ok": true });
        assert_eq!(get_str(&value, "name"), Some("Jorge".to_string()));
        assert_eq!(get_str(&value, "missing"), None);
        assert!(get_bool(&value, "adult_ok"));
        assert!(!get_bool(&value, "missing"));
    }
}
