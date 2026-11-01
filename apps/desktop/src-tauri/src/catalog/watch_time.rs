//! Tiempo visto de verdad, por perfil (F17, parte 1): lo que suma «Tu año en VIBE».
//!
//! No hace falta que el reproductor cuente nada. En películas y episodios sale del
//! progreso que ya manda cada pocos segundos: lo que avanzó la posición desde el aviso
//! anterior, si llegó hace poco y no avanzó más que el reloj (un salto adelante no es
//! tiempo visto). En el directo, de un aviso «sigo viendo» cada medio minuto.
//!
//! Se guarda por día y hora locales del aparato (`tz_offset_min`, el de
//! `Date.getTimezoneOffset()`), y con la clave de la nube del título para que, cuando viaje
//! con la cuenta, lo visto en el iPhone y en el PC se sume igual.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{now_epoch, queries, reads, Result};

/// Un aviso más viejo que esto ya no es de la misma sesión: no se cuenta el hueco.
const MAX_GAP_S: i64 = 90;
/// Margen sobre el reloj (los avisos no llegan exactos; a 1,25× avanza algo más).
const CLOCK_SLACK_S: i64 = 10;
/// Lo más que cuenta un aviso «sigo viendo» del directo.
const MAX_HEARTBEAT_CREDIT_S: i64 = 60;
const DEVICE_ID_KEY: &str = "device_id";

/// Segundos vistos entre el aviso de progreso anterior (`previous`: posición y cuándo) y
/// este. 0 si no son de la misma sesión, si se fue hacia atrás o si saltó adelante.
pub fn credit_from_progress(previous: Option<(i64, i64)>, position_s: i64, now: i64) -> i64 {
    let Some((previous_position, previous_at)) = previous else { return 0 };
    let elapsed = now - previous_at;
    let advanced = position_s - previous_position;
    if elapsed <= 0 || elapsed > MAX_GAP_S || advanced <= 0 || advanced > elapsed + CLOCK_SLACK_S {
        return 0;
    }
    advanced
}

/// Segundos vistos de un canal desde su aviso «sigo viendo» anterior.
pub fn credit_from_heartbeat(profile_id: i64, channel_id: i64, now: i64) -> i64 {
    static LAST: OnceLock<Mutex<HashMap<(i64, i64), i64>>> = OnceLock::new();
    let Ok(mut last) = LAST.get_or_init(Default::default).lock() else { return 0 };
    let previous = last.insert((profile_id, channel_id), now);
    match previous {
        Some(at) if now > at && now - at <= MAX_GAP_S => (now - at).min(MAX_HEARTBEAT_CREDIT_S),
        _ => 0,
    }
}

/// Fecha (`AAAA-MM-DD`) y hora locales de un instante, con el desfase del aparato en
/// minutos tal como lo da `Date.getTimezoneOffset()` (UTC − local: −60 en Madrid en invierno).
pub fn local_day_hour(epoch: i64, tz_offset_min: i32) -> (String, i64) {
    let offset = i64::from(tz_offset_min.clamp(-14 * 60, 14 * 60));
    let local = epoch - offset * 60;
    let days = local.div_euclid(86_400);
    let hour = local.rem_euclid(86_400) / 3600;
    let (year, month, day) = civil_from_days(days);
    (format!("{year:04}-{month:02}-{day:02}"), hour)
}

/// Días desde 1970-01-01 a fecha del calendario (algoritmo de Howard Hinnant).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Id de este aparato para lo visto (se crea la primera vez).
pub fn device_id(conn: &Connection) -> Result<String> {
    if let Some(id) = reads::get_setting(conn, DEVICE_ID_KEY)? {
        return Ok(id);
    }
    use rand::RngExt;
    let id = format!("{:016x}", rand::rng().random::<u64>());
    queries::set_setting(conn, DEVICE_ID_KEY, &id)?;
    Ok(id)
}

/// El nombre con el que se recordará lo visto: la película, la serie del episodio o el canal.
fn title_for(conn: &Connection, item_kind: &str, item_id: i64) -> Result<Option<String>> {
    let sql = match item_kind {
        "movie" => "SELECT clean_title FROM movies WHERE id = ?1",
        "episode" => {
            "SELECT s.clean_title FROM episodes e
             JOIN series_sources ss ON ss.id = e.series_source_id JOIN series s ON s.id = ss.series_id
             WHERE e.id = ?1"
        }
        "channel" => "SELECT display_name FROM channels WHERE id = ?1",
        _ => return Ok(None),
    };
    conn.query_row(sql, params![item_id], |row| row.get(0)).optional().map_err(Into::into)
}

/// Lo visto por un perfil en un aparato un día, en el JSON de la nube (`RemoteWatchDay`).
pub fn day_entries_json(conn: &Connection, profile_id: i64, device_id: &str, day: &str) -> Result<String> {
    let mut stmt = conn.prepare("SELECT hour, item_kind, item_key, title, seconds FROM watch_time WHERE profile_id = ?1 AND device_id = ?2 AND day = ?3")?;
    let entries = stmt
        .query_map(params![profile_id, device_id, day], |r| {
            Ok(serde_json::json!({ "h": r.get::<_, i64>(0)?, "k": r.get::<_, String>(1)?, "key": r.get::<_, String>(2)?, "t": r.get::<_, String>(3)?, "s": r.get::<_, i64>(4)? }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(serde_json::Value::Array(entries).to_string())
}

/// Los días con algo visto por un perfil en este aparato.
pub fn local_days(conn: &Connection, profile_id: i64, device_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT day FROM watch_time WHERE profile_id = ?1 AND device_id = ?2")?;
    let days = stmt.query_map(params![profile_id, device_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(days)
}

/// Guarda lo que otro aparato vio un día (llegado de la nube): sustituye lo que había.
pub fn replace_remote_day(conn: &Connection, profile_id: i64, device_id: &str, day: &str, entries_json: &str) -> Result<()> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(entries_json).unwrap_or_default();
    conn.execute("DELETE FROM watch_time WHERE profile_id = ?1 AND device_id = ?2 AND day = ?3", params![profile_id, device_id, day])?;
    let now = now_epoch();
    for entry in entries {
        let (Some(hour), Some(kind), Some(key), Some(title), Some(seconds)) =
            (entry["h"].as_i64(), entry["k"].as_str(), entry["key"].as_str(), entry["t"].as_str(), entry["s"].as_i64())
        else {
            continue;
        };
        if !(0..24).contains(&hour) || !matches!(kind, "movie" | "episode" | "channel") || seconds <= 0 {
            continue;
        }
        conn.execute(
            "INSERT OR REPLACE INTO watch_time (profile_id, device_id, day, hour, item_kind, item_key, title, seconds, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![profile_id, device_id, day, hour, kind, key, title, seconds, now],
        )?;
    }
    Ok(())
}

/// Suma `seconds` vistos de un título a este perfil, en la hora local de `at`.
pub fn add(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64, seconds: i64, at: i64, tz_offset_min: i32) -> Result<()> {
    if seconds <= 0 {
        return Ok(());
    }
    let (Some(item_key), Some(title)) = (reads::item_key_for(conn, item_kind, item_id)?, title_for(conn, item_kind, item_id)?) else {
        return Ok(());
    };
    let device = device_id(conn)?;
    let (day, hour) = local_day_hour(at, tz_offset_min);
    conn.execute(
        "INSERT INTO watch_time (profile_id, device_id, day, hour, item_kind, item_key, title, seconds, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(profile_id, device_id, day, hour, item_kind, item_key) DO UPDATE SET
            seconds = seconds + excluded.seconds, title = excluded.title, updated_at = excluded.updated_at",
        params![profile_id, device, day, hour, item_kind, item_key, title, seconds, now_epoch()],
    )?;
    // Viaja con la cuenta (agrupado): «Tu año» suma todos los aparatos del perfil.
    crate::sync::outbox::enqueue_watch_day(conn, profile_id, &device, &day)?;
    Ok(())
}

/// Posición y momento del último progreso guardado, para `credit_from_progress`.
pub fn previous_progress(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT position_s, played_at FROM playback_state WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
        params![profile_id, item_kind, item_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_only_real_viewing() {
        let now = 1_800_000_000;
        assert_eq!(credit_from_progress(None, 100, now), 0, "primer aviso: nada que comparar");
        assert_eq!(credit_from_progress(Some((90, now - 10)), 100, now), 10);
        assert_eq!(credit_from_progress(Some((90, now - 10)), 104, now), 14, "a algo más de velocidad, con margen");
        assert_eq!(credit_from_progress(Some((90, now - 10)), 900, now), 0, "un salto adelante no es tiempo visto");
        assert_eq!(credit_from_progress(Some((900, now - 10)), 100, now), 0, "hacia atrás tampoco");
        assert_eq!(credit_from_progress(Some((90, now - 600)), 100, now), 0, "de otra sesión");
        assert_eq!(credit_from_progress(Some((100, now - 10)), 100, now), 0, "en pausa");
    }

    #[test]
    fn heartbeats_count_the_time_between_them() {
        let now = 1_800_000_000;
        assert_eq!(credit_from_heartbeat(7, 1, now), 0);
        assert_eq!(credit_from_heartbeat(7, 1, now + 30), 30);
        assert_eq!(credit_from_heartbeat(7, 1, now + 1000), 0, "volvió mucho después: no se cuenta el hueco");
        assert_eq!(credit_from_heartbeat(8, 1, now + 1030), 0, "cada perfil por su lado");
    }

    #[test]
    fn local_day_and_hour_follow_the_device_offset() {
        // 2026-11-01 23:30 UTC: en Madrid (UTC+1, offset −60) ya es día 2 a las 0:30.
        let at = 1_793_575_800;
        assert_eq!(local_day_hour(at, 0), ("2026-11-01".to_string(), 23));
        assert_eq!(local_day_hour(at, -60), ("2026-11-02".to_string(), 0));
        assert_eq!(local_day_hour(at, 300), ("2026-11-01".to_string(), 18), "Nueva York");
        assert_eq!(local_day_hour(951_782_400, 0), ("2000-02-29".to_string(), 0), "año bisiesto");
        assert_eq!(local_day_hour(0, 0), ("1970-01-01".to_string(), 0));
    }

    #[test]
    fn adds_up_by_hour_and_title() {
        let conn = crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO profiles (id, name, created_at) VALUES (1, 'Ana', 0);
             INSERT INTO movies (id, clean_title, match_key) VALUES (1, 'Peli', 'peli-2020');",
        )
        .unwrap();
        let at = 1_793_575_800;
        add(&conn, 1, "movie", 1, 10, at, 0).unwrap();
        add(&conn, 1, "movie", 1, 20, at + 60, 0).unwrap();
        add(&conn, 1, "movie", 999, 20, at, 0).unwrap(); // no existe: no se apunta
        let rows: Vec<(String, i64, String, String, i64)> = conn
            .prepare("SELECT day, hour, item_key, title, seconds FROM watch_time")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, vec![("2026-11-01".to_string(), 23, "peli-2020".to_string(), "Peli".to_string(), 30)]);
        assert_eq!(device_id(&conn).unwrap(), device_id(&conn).unwrap(), "el id del aparato no cambia");
    }
}
