//! Fuentes de una película o un episodio: cuál va primero y a cuál se pasa si una falla o
//! va lenta (F17, parte 1). Lo deciden el PC y el iPhone igual, porque se decide aquí.
//!
//! Se guarda cómo ha ido cada fuente (`source_health`): si arrancó, cuánto tardó y si el
//! reproductor avisó de que iba lenta. Una fuente que acaba de fallar o de ir lenta baja
//! al final, y también las de un proveedor que está fallando ahora; a igualdad, el orden de
//! siempre (el del catálogo). Por `provider_id` y `remote_id`, nunca por URL (R5).

use rusqlite::{params, Connection, OptionalExtension};

use super::normalize;
use crate::db::{now_epoch, Result};

/// Un fallo cuenta durante este tiempo si la fuente no ha vuelto a arrancar después.
const RECENT_FAILURE_S: i64 = 6 * 3600;
/// Una fuente lenta lo sigue siendo un día (el mismo fichero en el mismo servidor).
const RECENT_SLOW_S: i64 = 24 * 3600;
/// Fallos de un proveedor en este rato, sin ningún arranque bueno después: está caído.
const PROVIDER_TROUBLE_S: i64 = 30 * 60;
/// Un arranque así de lento ya es peor que el de otra fuente normal.
const SLOW_START_MS: i64 = 8_000;

/// Lo guardado de una fuente.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceHealth {
    pub last_start_ms: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_failure_at: Option<i64>,
    pub last_slow_at: Option<i64>,
}

/// Cómo van las fuentes de un proveedor ahora mismo.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProviderHealth {
    pub recent_failures: i64,
    pub last_success_at: Option<i64>,
    pub last_failure_at: Option<i64>,
}

/// Cómo acabó un intento de abrir una fuente.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartOutcome {
    /// Arrancó; `ms` es lo que tardó en dar los primeros datos (solo al empezar).
    Started { ms: Option<i64> },
    Failed,
}

fn failed_last(last_failure_at: Option<i64>, last_success_at: Option<i64>) -> bool {
    match (last_failure_at, last_success_at) {
        (Some(failure), Some(success)) => failure > success,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Cuanto más alto, más abajo va la fuente. 0 es una fuente sin problemas conocidos.
pub fn penalty(source: Option<&SourceHealth>, provider: Option<&ProviderHealth>, now: i64) -> i64 {
    let mut penalty = 0;
    if let Some(health) = source {
        if failed_last(health.last_failure_at, health.last_success_at) && health.last_failure_at.is_some_and(|at| now - at < RECENT_FAILURE_S) {
            penalty += 100;
        }
        if health.last_slow_at.is_some_and(|at| now - at < RECENT_SLOW_S) {
            penalty += 40;
        }
        if health.last_start_ms.is_some_and(|ms| ms > SLOW_START_MS) {
            penalty += 10;
        }
    }
    if let Some(provider) = provider {
        if provider.recent_failures >= 2 && failed_last(provider.last_failure_at, provider.last_success_at) {
            penalty += 60;
        }
    }
    penalty
}

/// Lo guardado de una fuente, si alguna vez se abrió.
pub fn source_health(conn: &Connection, provider_id: i64, item_kind: &str, remote_id: &str) -> Result<Option<SourceHealth>> {
    conn.query_row(
        "SELECT last_start_ms, last_success_at, last_failure_at, last_slow_at FROM source_health
         WHERE provider_id = ?1 AND item_kind = ?2 AND remote_id = ?3",
        params![provider_id, item_kind, remote_id],
        |row| Ok(SourceHealth { last_start_ms: row.get(0)?, last_success_at: row.get(1)?, last_failure_at: row.get(2)?, last_slow_at: row.get(3)? }),
    )
    .optional()
    .map_err(Into::into)
}

/// Cómo van ahora las fuentes de un proveedor (películas y episodios juntos).
pub fn provider_health(conn: &Connection, provider_id: i64, now: i64) -> Result<ProviderHealth> {
    conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN last_failure_at > ?2 THEN 1 ELSE 0 END), 0), MAX(last_success_at), MAX(last_failure_at)
         FROM source_health WHERE provider_id = ?1",
        params![provider_id, now - PROVIDER_TROUBLE_S],
        |row| Ok(ProviderHealth { recent_failures: row.get(0)?, last_success_at: row.get(1)?, last_failure_at: row.get(2)? }),
    )
    .map_err(Into::into)
}

/// Apunta cómo acabó un intento de abrir una fuente.
pub fn record_start(conn: &Connection, provider_id: i64, item_kind: &str, remote_id: &str, outcome: StartOutcome) -> Result<()> {
    let now = now_epoch();
    match outcome {
        StartOutcome::Started { ms } => conn.execute(
            "INSERT INTO source_health (provider_id, item_kind, remote_id, successes, last_start_ms, last_success_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5)
             ON CONFLICT(provider_id, item_kind, remote_id) DO UPDATE SET
                successes = successes + 1,
                last_start_ms = COALESCE(excluded.last_start_ms, last_start_ms),
                last_success_at = excluded.last_success_at",
            params![provider_id, item_kind, remote_id, ms, now],
        )?,
        StartOutcome::Failed => conn.execute(
            "INSERT INTO source_health (provider_id, item_kind, remote_id, failures, last_failure_at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(provider_id, item_kind, remote_id) DO UPDATE SET
                failures = failures + 1, last_failure_at = excluded.last_failure_at",
            params![provider_id, item_kind, remote_id, now],
        )?,
    };
    Ok(())
}

/// El reproductor avisó de que esta fuente va lenta (tarda en arrancar o se para).
pub fn record_slow(conn: &Connection, provider_id: i64, item_kind: &str, remote_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO source_health (provider_id, item_kind, remote_id, slow_reports, last_slow_at)
         VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(provider_id, item_kind, remote_id) DO UPDATE SET
            slow_reports = slow_reports + 1, last_slow_at = excluded.last_slow_at",
        params![provider_id, item_kind, remote_id, now_epoch()],
    )?;
    Ok(())
}

/// Ordena `items` de mejor a peor fuente. Estable: a igualdad, el orden que traían.
pub fn rank<T>(conn: &Connection, item_kind: &str, items: &mut Vec<T>, key: impl Fn(&T) -> (i64, &str)) -> Result<()> {
    if items.len() < 2 {
        return Ok(());
    }
    let now = now_epoch();
    let mut providers: Vec<(i64, ProviderHealth)> = Vec::new();
    let mut penalties = Vec::with_capacity(items.len());
    for item in items.iter() {
        let (provider_id, remote_id) = key(item);
        let provider = match providers.iter().find(|(id, _)| *id == provider_id) {
            Some((_, health)) => *health,
            None => {
                let health = provider_health(conn, provider_id, now)?;
                providers.push((provider_id, health));
                health
            }
        };
        let source = source_health(conn, provider_id, item_kind, remote_id)?;
        penalties.push(penalty(source.as_ref(), Some(&provider), now));
    }
    let mut tagged: Vec<(i64, T)> = penalties.into_iter().zip(items.drain(..)).collect();
    tagged.sort_by_key(|(penalty, _)| *penalty);
    items.extend(tagged.into_iter().map(|(_, item)| item));
    Ok(())
}

/// La mejor fuente de una película, la que se abre si no se pide ninguna.
pub fn best_movie_source(conn: &Connection, movie_id: i64) -> Result<Option<i64>> {
    let mut stmt = conn.prepare("SELECT id, provider_id, remote_id FROM movie_sources WHERE movie_id = ?1 ORDER BY id")?;
    let mut sources = stmt
        .query_map(params![movie_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rank(conn, "movie", &mut sources, |(_, provider_id, remote_id)| (*provider_id, remote_id.as_str()))?;
    Ok(sources.first().map(|(id, _, _)| *id))
}

/// Una fuente de un episodio: el mismo episodio (temporada y número) en otra entrada de la
/// misma serie del proveedor (otra calidad, otro idioma u otro proveedor).
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeSource {
    /// Id del episodio en esa entrada (es lo que se pide como `source`).
    pub id: i64,
    pub provider_id: i64,
    pub remote_id: String,
    pub quality: Option<String>,
    pub language: Option<String>,
}

/// Todas las fuentes del episodio, la suya incluida, de mejor a peor.
pub fn episode_sources(conn: &Connection, episode_id: i64) -> Result<Vec<EpisodeSource>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, ss.provider_id, e.remote_id, ss.raw_title
         FROM episodes me
         JOIN series_sources mss ON mss.id = me.series_source_id
         JOIN series_sources ss ON ss.series_id = mss.series_id
         JOIN episodes e ON e.series_source_id = ss.id AND e.season = me.season AND e.number = me.number
         WHERE me.id = ?1
         ORDER BY (e.id != ?1), ss.id, e.id",
    )?;
    let mut sources = stmt
        .query_map(params![episode_id], |row| {
            let raw_title: String = row.get(3)?;
            let normalized = normalize::normalize(&raw_title);
            Ok(EpisodeSource { id: row.get(0)?, provider_id: row.get(1)?, remote_id: row.get(2)?, quality: normalized.quality, language: normalized.language })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Una entrada repetida del mismo episodio (pasa en algunos paneles) no es otra fuente.
    let mut seen = std::collections::HashSet::new();
    sources.retain(|source| seen.insert((source.provider_id, source.remote_id.clone())));
    rank(conn, "episode", &mut sources, |source| (source.provider_id, source.remote_id.as_str()))?;
    Ok(sources)
}

/// Las otras entradas de la serie de este episodio cuyos episodios no se han pedido nunca
/// al proveedor: hasta pedirlos no se sabe si tienen este episodio.
pub fn series_sources_without_detail(conn: &Connection, episode_id: i64) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT ss.id, ss.provider_id
         FROM episodes me
         JOIN series_sources mss ON mss.id = me.series_source_id
         JOIN series_sources ss ON ss.series_id = mss.series_id AND ss.id != mss.id
         WHERE me.id = ?1 AND ss.detail_synced_at IS NULL
         ORDER BY ss.id",
    )?;
    let rows = stmt.query_map(params![episode_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_source_that_just_failed_goes_last_until_it_starts_again() {
        let failed = SourceHealth { last_failure_at: Some(NOW - 60), ..Default::default() };
        assert!(penalty(Some(&failed), None, NOW) >= 100);
        let recovered = SourceHealth { last_failure_at: Some(NOW - 60), last_success_at: Some(NOW - 10), ..Default::default() };
        assert_eq!(penalty(Some(&recovered), None, NOW), 0);
        let old_failure = SourceHealth { last_failure_at: Some(NOW - RECENT_FAILURE_S - 1), ..Default::default() };
        assert_eq!(penalty(Some(&old_failure), None, NOW), 0);
    }

    #[test]
    fn slow_sources_and_failing_providers_go_down() {
        let slow = SourceHealth { last_slow_at: Some(NOW - 3600), last_success_at: Some(NOW - 3600), ..Default::default() };
        let slow_start = SourceHealth { last_start_ms: Some(12_000), ..Default::default() };
        assert!(penalty(Some(&slow), None, NOW) > penalty(Some(&slow_start), None, NOW));
        assert!(penalty(Some(&slow_start), None, NOW) > penalty(None, None, NOW));

        let down = ProviderHealth { recent_failures: 3, last_failure_at: Some(NOW - 30), last_success_at: Some(NOW - 600) };
        assert_eq!(penalty(None, Some(&down), NOW), 60);
        // Un solo fallo, o uno con un arranque bueno después, no tumba al proveedor.
        let one = ProviderHealth { recent_failures: 1, last_failure_at: Some(NOW - 30), last_success_at: None };
        let back = ProviderHealth { recent_failures: 3, last_failure_at: Some(NOW - 30), last_success_at: Some(NOW - 5) };
        assert_eq!(penalty(None, Some(&one), NOW), 0);
        assert_eq!(penalty(None, Some(&back), NOW), 0);
    }

    fn test_db() -> Connection {
        let conn = crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO providers (id, name, kind, base_url, created_at) VALUES (1, 'P', 'xtream', 'http://x', 0), (2, 'Q', 'xtream', 'http://y', 0);
             INSERT INTO movies (id, clean_title, match_key) VALUES (1, 'Peli', 'peli-0000');
             INSERT INTO movie_sources (id, movie_id, provider_id, remote_id, raw_title) VALUES (10, 1, 1, 'a', 'Peli 1080p'), (11, 1, 1, 'b', 'Peli 720p'), (12, 1, 2, 'c', 'Peli');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn the_best_movie_source_skips_failed_ones_and_failing_providers() {
        let conn = test_db();
        assert_eq!(best_movie_source(&conn, 1).unwrap(), Some(10), "sin historial, el orden de siempre");
        record_start(&conn, 1, "movie", "a", StartOutcome::Failed).unwrap();
        assert_eq!(best_movie_source(&conn, 1).unwrap(), Some(11));
        record_start(&conn, 1, "movie", "b", StartOutcome::Failed).unwrap();
        // Dos fallos seguidos del proveedor 1: la del proveedor 2 primero.
        assert_eq!(best_movie_source(&conn, 1).unwrap(), Some(12));
        record_start(&conn, 1, "movie", "a", StartOutcome::Started { ms: Some(900) }).unwrap();
        record_start(&conn, 1, "movie", "b", StartOutcome::Started { ms: Some(900) }).unwrap();
        assert_eq!(best_movie_source(&conn, 1).unwrap(), Some(10), "volvió a arrancar: vuelve a su sitio");
        record_slow(&conn, 1, "movie", "a").unwrap();
        assert_eq!(best_movie_source(&conn, 1).unwrap(), Some(11));
    }

    #[test]
    fn episode_sources_are_the_same_episode_in_other_entries_of_the_series() {
        let conn = test_db();
        conn.execute_batch(
            "INSERT INTO series (id, clean_title, match_key) VALUES (1, 'Serie', 'serie-0000');
             INSERT INTO series_sources (id, series_id, provider_id, remote_id, raw_title, detail_synced_at) VALUES
               (1, 1, 1, 's1', 'ES | Serie', 1), (2, 1, 1, 's2', 'ES | Serie 4K', 1), (3, 1, 2, 's3', 'Serie', NULL),
               (4, 1, 1, 's4', 'Serie (copia)', 1);
             INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES
               (100, 1, 'e1', 1, 1), (101, 1, 'e2', 1, 2), (200, 2, 'f1', 1, 1), (400, 4, 'f1', 1, 1);",
        )
        .unwrap();
        let sources = episode_sources(&conn, 100).unwrap();
        assert_eq!(sources.iter().map(|s| s.id).collect::<Vec<_>>(), vec![100, 200], "el propio primero, sin repetidos");
        assert_eq!(sources[1].quality.as_deref(), Some("4K"));
        assert_eq!(episode_sources(&conn, 101).unwrap().len(), 1);
        assert_eq!(series_sources_without_detail(&conn, 100).unwrap(), vec![(3, 2)]);

        record_start(&conn, 1, "episode", "e1", StartOutcome::Failed).unwrap();
        assert_eq!(episode_sources(&conn, 100).unwrap()[0].id, 200, "la que falló, al final");
    }
}
