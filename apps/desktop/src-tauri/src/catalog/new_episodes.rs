//! Avisos de episodios nuevos (F17, parte 4), activables a mano en cada aparato y perfil
//! (apagados de serie).
//!
//! Series seguidas: las de «Mi lista» y las que el perfil ha visto en los últimos 90 días.
//! Cada 6 horas como mucho, y nunca mientras se reproduce algo (R1), se piden al proveedor
//! los episodios de unas pocas (`MAX_SERIES_PER_CHECK`, una petición cada una y espaciadas:
//! el panel del usuario bloquea si se le pregunta mucho). Hay episodio nuevo cuando el más
//! alto (temporada, número) supera al que se conocía; la primera vez solo se apunta.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{now_epoch, queries, reads, schema, Result};
use crate::util::stream_guard::StreamGuard;

/// Lo más a menudo que se comprueba.
pub const CHECK_EVERY_S: i64 = 6 * 3600;
/// Series que se piden al proveedor en cada comprobación.
const MAX_SERIES_PER_CHECK: usize = 15;
/// Entre una petición y otra.
const PAUSE_BETWEEN: Duration = Duration::from_millis(1500);
/// Visto hace menos de esto: la serie se sigue.
const RECENT_S: i64 = 90 * 86_400;
const CHECKED_AT_KEY: &str = "new_episodes_checked_at";

fn setting_key(profile_id: i64) -> String {
    format!("new_episodes_alerts:{profile_id}")
}

/// Si este perfil quiere avisos de episodios nuevos en este aparato.
pub fn enabled(conn: &Connection, profile_id: i64) -> Result<bool> {
    Ok(reads::get_setting(conn, &setting_key(profile_id))?.as_deref() == Some("1"))
}

/// Activa o desactiva los avisos. Al activarlos se apunta lo que hay ahora, para no avisar
/// de golpe de todo lo que ya existía.
pub fn set_enabled(conn: &Connection, profile_id: i64, on: bool) -> Result<()> {
    queries::set_setting(conn, &setting_key(profile_id), if on { "1" } else { "0" })?;
    if on {
        let now = now_epoch();
        for series_id in followed_series(conn, profile_id, now)? {
            if let Some((season, number)) = latest_episode(conn, series_id)? {
                mark(conn, profile_id, series_id, season, number, now)?;
            }
        }
    }
    Ok(())
}

fn enabled_profiles(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM profiles ORDER BY id")?;
    let ids = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for id in ids {
        if enabled(conn, id)? {
            out.push(id);
        }
    }
    Ok(out)
}

/// Las series que sigue un perfil: «Mi lista» y lo visto hace poco, lo más reciente primero.
pub fn followed_series(conn: &Connection, profile_id: i64, now: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT series_id, MAX(at) FROM (
            SELECT f.item_id AS series_id, f.added_at AS at FROM favorites f WHERE f.profile_id = ?1 AND f.item_kind = 'series'
            UNION ALL
            SELECT ss.series_id, p.played_at FROM playback_state p
              JOIN episodes e ON e.id = p.item_id JOIN series_sources ss ON ss.id = e.series_source_id
             WHERE p.profile_id = ?1 AND p.item_kind = 'episode' AND p.played_at > ?2
         ) GROUP BY series_id ORDER BY MAX(at) DESC",
    )?;
    let rows = stmt.query_map(params![profile_id, now - RECENT_S], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// El episodio más alto que se conoce de la serie, entre todas sus entradas.
pub fn latest_episode(conn: &Connection, series_id: i64) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT e.season, e.number FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id
         WHERE ss.series_id = ?1 ORDER BY e.season DESC, e.number DESC LIMIT 1",
        params![series_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn mark(conn: &Connection, profile_id: i64, series_id: i64, season: i64, number: i64, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO new_episode_marks (profile_id, series_id, season, number, checked_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(profile_id, series_id) DO UPDATE SET season = excluded.season, number = excluded.number, checked_at = excluded.checked_at",
        params![profile_id, series_id, season, number, now],
    )?;
    Ok(())
}

/// Un aviso que dar: «Nuevo episodio de X», «T2 · E5».
#[derive(Debug, Clone, PartialEq)]
pub struct NewEpisode {
    pub profile_id: i64,
    pub series_id: i64,
    pub series_title: String,
    pub season: i64,
    pub number: i64,
}

/// Compara lo que se conoce ahora con lo apuntado y apunta lo nuevo. Devuelve los avisos.
pub fn detect(conn: &Connection, profile_id: i64, now: i64) -> Result<Vec<NewEpisode>> {
    let mut out = Vec::new();
    for series_id in followed_series(conn, profile_id, now)? {
        let Some((season, number)) = latest_episode(conn, series_id)? else { continue };
        let previous: Option<(i64, i64)> = conn
            .query_row("SELECT season, number FROM new_episode_marks WHERE profile_id = ?1 AND series_id = ?2", params![profile_id, series_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if previous.is_some_and(|prev| (season, number) > prev) {
            let title: String = conn.query_row("SELECT clean_title FROM series WHERE id = ?1", params![series_id], |r| r.get(0))?;
            out.push(NewEpisode { profile_id, series_id, series_title: title, season, number });
        }
        if previous != Some((season, number)) {
            mark(conn, profile_id, series_id, season, number, now)?;
        }
    }
    Ok(out)
}

/// Las entradas del proveedor a pedir: una por serie seguida (la más vieja), las primeras
/// `MAX_SERIES_PER_CHECK` entre todos los perfiles con avisos.
fn sources_to_refresh(conn: &Connection, profiles: &[i64], now: i64) -> Result<Vec<(i64, i64)>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for profile_id in profiles {
        for series_id in followed_series(conn, *profile_id, now)? {
            if !seen.insert(series_id) {
                continue;
            }
            let source: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT id, provider_id FROM series_sources WHERE series_id = ?1 ORDER BY detail_synced_at NULLS FIRST, id LIMIT 1",
                    params![series_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some(source) = source {
                out.push(source);
            }
            if out.len() >= MAX_SERIES_PER_CHECK {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Una comprobación: si toca (o `force`), pide al proveedor las series seguidas y devuelve
/// los episodios nuevos. Se para si empieza una reproducción.
pub async fn check(db_path: PathBuf, guard: Arc<StreamGuard>, force: bool) -> Result<Vec<NewEpisode>> {
    let now = now_epoch();
    let (profiles, due, sources) = {
        let path = db_path.clone();
        tokio::task::spawn_blocking(move || -> Result<(Vec<i64>, bool, Vec<(i64, i64)>)> {
            let conn = schema::open_readonly_at(&path)?;
            let profiles = enabled_profiles(&conn)?;
            let last: i64 = reads::get_setting(&conn, CHECKED_AT_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
            let sources = if profiles.is_empty() { Vec::new() } else { sources_to_refresh(&conn, &profiles, now)? };
            Ok((profiles, now - last >= CHECK_EVERY_S, sources))
        })
        .await
        .map_err(|err| crate::db::DbError::Io(std::io::Error::other(err.to_string())))??
    };
    if profiles.is_empty() || !(due || force) {
        return Ok(Vec::new());
    }
    let mut asked = 0;
    for (series_source_id, provider_id) in sources {
        if guard.is_busy() {
            tracing::info!("episodios nuevos: se deja para luego, hay algo reproduciéndose");
            return Ok(Vec::new());
        }
        if asked > 0 {
            tokio::time::sleep(PAUSE_BETWEEN).await;
        }
        asked += 1;
        if let Err(err) = crate::server::api::refresh_series_detail(db_path.clone(), series_source_id, provider_id).await {
            tracing::warn!(series_source_id, error = %err, "episodios nuevos: no se pudo pedir la serie");
        }
    }
    let found = tokio::task::spawn_blocking(move || -> Result<Vec<NewEpisode>> {
        let conn = schema::open_readonly_at(&db_path)?;
        let mut found = Vec::new();
        for profile_id in profiles {
            found.extend(detect(&conn, profile_id, now)?);
        }
        queries::set_setting(&conn, CHECKED_AT_KEY, &now.to_string())?;
        Ok(found)
    })
    .await
    .map_err(|err| crate::db::DbError::Io(std::io::Error::other(err.to_string())))??;
    tracing::info!(asked, nuevos = found.len(), "episodios nuevos comprobados");
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> Connection {
        let conn = schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO providers (id, name, kind, base_url, created_at) VALUES (1, 'P', 'xtream', 'http://x', 0);
             INSERT INTO profiles (id, name, created_at) VALUES (1, 'Ana', 0);
             INSERT INTO series (id, clean_title, match_key) VALUES (1, 'Serie', 'serie-2020'), (2, 'Otra', 'otra-2020');
             INSERT INTO series_sources (id, series_id, provider_id, remote_id, raw_title) VALUES (10, 1, 1, 's1', 'Serie'), (20, 2, 1, 's2', 'Otra');
             INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES (100, 10, 'a', 1, 1), (101, 10, 'b', 1, 2), (200, 20, 'c', 1, 1);
             INSERT INTO favorites (profile_id, item_kind, item_id, added_at) VALUES (1, 'series', 1, 10);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn only_new_episodes_of_followed_series_and_never_on_the_first_look() {
        let conn = seed();
        let now = now_epoch();
        assert!(!enabled(&conn, 1).unwrap());
        set_enabled(&conn, 1, true).unwrap();
        assert!(detect(&conn, 1, now).unwrap().is_empty(), "lo que ya había no avisa");

        conn.execute("INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES (102, 10, 'z', 2, 1)", []).unwrap();
        let found = detect(&conn, 1, now).unwrap();
        assert_eq!(found.iter().map(|n| (n.series_title.as_str(), n.season, n.number)).collect::<Vec<_>>(), vec![("Serie", 2, 1)]);
        assert!(detect(&conn, 1, now).unwrap().is_empty(), "una sola vez");

        // El mismo episodio subido otra vez con otro id no es nuevo.
        conn.execute("INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES (103, 10, 'z2', 2, 1)", []).unwrap();
        assert!(detect(&conn, 1, now).unwrap().is_empty());

        // «Otra» no se sigue: aunque gane un episodio, no avisa.
        conn.execute("INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES (201, 20, 'd', 1, 2)", []).unwrap();
        assert!(detect(&conn, 1, now).unwrap().is_empty());

        // Verla la hace seguida.
        conn.execute("INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, played_at) VALUES (1, 'episode', 200, 60, ?1)", params![now]).unwrap();
        assert_eq!(followed_series(&conn, 1, now).unwrap(), vec![2, 1]);
    }
}
