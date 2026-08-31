//! Consultas tipadas contra SQLite; ningún SQL suelto fuera de este módulo.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::sources::model::{MediaKind, RawCategory, RawChannel, RawMovie, RawSeries};

use super::{now_epoch, Result};

/// Tipo de proveedor, igual que la columna `kind` de `providers` en `docs/DATA-MODEL.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "api-types.ts")]
pub enum ProviderKind {
    Xtream,
    M3u,
}

impl ProviderKind {
    fn as_db_str(self) -> &'static str {
        match self {
            ProviderKind::Xtream => "xtream",
            ProviderKind::M3u => "m3u",
        }
    }

    fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "xtream" => Some(Self::Xtream),
            "m3u" => Some(Self::M3u),
            _ => None,
        }
    }
}

/// Fila cruda de `providers`. `credential_ref` no debe salir de la capa de datos hacia
/// la interfaz: solo hace falta saber que existe, no su valor (regla R5).
#[derive(Debug, Clone)]
pub struct ProviderRow {
    pub id: i64,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub username: Option<String>,
    pub credential_ref: Option<String>,
    pub max_connections: i64,
    pub expires_at: Option<i64>,
    pub enabled: bool,
    pub last_sync_at: Option<i64>,
    pub created_at: i64,
}

fn row_to_provider(row: &Row) -> rusqlite::Result<ProviderRow> {
    let kind_str: String = row.get("kind")?;
    Ok(ProviderRow {
        id: row.get("id")?,
        name: row.get("name")?,
        kind: ProviderKind::from_db_str(&kind_str).unwrap_or(ProviderKind::Xtream),
        base_url: row.get("base_url")?,
        username: row.get("username")?,
        credential_ref: row.get("credential_ref")?,
        max_connections: row.get("max_connections")?,
        expires_at: row.get("expires_at")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        last_sync_at: row.get("last_sync_at")?,
        created_at: row.get("created_at")?,
    })
}

pub fn insert_provider(
    conn: &Connection,
    kind: ProviderKind,
    name: &str,
    base_url: &str,
    username: Option<&str>,
    max_connections: i64,
    expires_at: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO providers (name, kind, base_url, username, max_connections, expires_at, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
        params![name, kind.as_db_str(), base_url, username, max_connections, expires_at, now_epoch()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_provider_credential_ref(conn: &Connection, provider_id: i64, credential_ref: &str) -> Result<()> {
    conn.execute(
        "UPDATE providers SET credential_ref = ?1 WHERE id = ?2",
        params![credential_ref, provider_id],
    )?;
    Ok(())
}

pub fn update_provider_account_status(
    conn: &Connection,
    provider_id: i64,
    max_connections: i64,
    expires_at: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE providers SET max_connections = ?1, expires_at = ?2 WHERE id = ?3",
        params![max_connections, expires_at, provider_id],
    )?;
    Ok(())
}

pub fn mark_provider_synced(conn: &Connection, provider_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE providers SET last_sync_at = ?1 WHERE id = ?2",
        params![now_epoch(), provider_id],
    )?;
    Ok(())
}

pub fn get_provider(conn: &Connection, id: i64) -> Result<Option<ProviderRow>> {
    conn.query_row("SELECT * FROM providers WHERE id = ?1", params![id], row_to_provider)
        .optional()
        .map_err(Into::into)
}

pub fn list_providers(conn: &Connection) -> Result<Vec<ProviderRow>> {
    let mut stmt = conn.prepare("SELECT * FROM providers ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], row_to_provider)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Campos ya calculados por `catalog::normalize` para una entrada. `queries.rs` no
/// depende de `catalog`: recibe estos valores ya hechos, nunca los calcula (lógica de
/// negocio en `catalog::sync`, SQL aquí).
#[derive(Clone, Copy)]
pub struct TitleFields<'a> {
    pub clean_title: &'a str,
    pub year: Option<i32>,
    pub match_key: &'a str,
    pub quality: Option<&'a str>,
    pub language: Option<&'a str>,
}

fn media_kind_db(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Live => "live",
        MediaKind::Movie => "movie",
        MediaKind::Series => "series",
    }
}

/// Upsert de categorías de un tipo, y mapa `remote_id -> id local` para enlazar
/// canales/películas con su categoría al insertarlos.
pub fn upsert_categories(
    conn: &Connection,
    provider_id: i64,
    kind: MediaKind,
    categories: &[RawCategory],
) -> Result<HashMap<String, i64>> {
    let kind_str = media_kind_db(kind);
    {
        let mut stmt = conn.prepare(
            "INSERT INTO categories (provider_id, remote_id, kind, name) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id, kind, remote_id) DO UPDATE SET name = excluded.name",
        )?;
        for category in categories {
            stmt.execute(params![provider_id, category.remote_id, kind_str, category.name])?;
        }
    }

    let mut map = HashMap::new();
    let mut select = conn.prepare("SELECT id, remote_id FROM categories WHERE provider_id = ?1 AND kind = ?2")?;
    let mut rows = select.query(params![provider_id, kind_str])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let remote_id: String = row.get(1)?;
        map.insert(remote_id, id);
    }
    Ok(map)
}

pub fn upsert_channel(
    conn: &Connection,
    provider_id: i64,
    category_map: &HashMap<String, i64>,
    channel: &RawChannel,
    fields: &TitleFields,
) -> Result<()> {
    let category_id = channel.category_remote_id.as_ref().and_then(|id| category_map.get(id).copied());
    conn.execute(
        "INSERT INTO channels
            (provider_id, remote_id, category_id, raw_name, display_name, logo_url, epg_channel_id,
             quality, language, has_catchup, catchup_days, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            category_id = excluded.category_id,
            raw_name = excluded.raw_name,
            display_name = excluded.display_name,
            logo_url = excluded.logo_url,
            epg_channel_id = excluded.epg_channel_id,
            quality = excluded.quality,
            language = excluded.language,
            has_catchup = excluded.has_catchup,
            catchup_days = excluded.catchup_days,
            sort_order = excluded.sort_order",
        params![
            provider_id,
            channel.remote_id,
            category_id,
            channel.raw_name,
            fields.clean_title,
            channel.logo_url,
            channel.epg_channel_id,
            fields.quality,
            fields.language,
            channel.has_catchup as i64,
            channel.catchup_days,
            channel.sort_order,
        ],
    )?;
    Ok(())
}

pub fn upsert_movie(
    conn: &Connection,
    provider_id: i64,
    category_map: &HashMap<String, i64>,
    movie: &RawMovie,
    fields: &TitleFields,
) -> Result<()> {
    conn.execute(
        "INSERT INTO movies (clean_title, year, match_key, tmdb_id, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(match_key) DO UPDATE SET clean_title = excluded.clean_title, year = excluded.year",
        params![fields.clean_title, fields.year, fields.match_key, movie.tmdb_id, movie.added_at],
    )?;
    let movie_id: i64 =
        conn.query_row("SELECT id FROM movies WHERE match_key = ?1", params![fields.match_key], |row| row.get(0))?;

    let category_id = movie.category_remote_id.as_ref().and_then(|id| category_map.get(id).copied());
    conn.execute(
        "INSERT INTO movie_sources (movie_id, provider_id, remote_id, category_id, raw_title, container, quality, language)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            movie_id = excluded.movie_id,
            category_id = excluded.category_id,
            raw_title = excluded.raw_title,
            container = excluded.container,
            quality = excluded.quality,
            language = excluded.language",
        params![
            movie_id,
            provider_id,
            movie.remote_id,
            category_id,
            movie.raw_title,
            movie.container_extension,
            fields.quality,
            fields.language
        ],
    )?;
    Ok(())
}

/// `series_sources` no tiene columna de categoría ni de calidad/idioma (ver
/// `docs/DATA-MODEL.md`): esas variantes son por episodio/fuente, no por serie.
pub fn upsert_series(conn: &Connection, provider_id: i64, series: &RawSeries, fields: &TitleFields) -> Result<()> {
    conn.execute(
        "INSERT INTO series (clean_title, year, match_key, added_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(match_key) DO UPDATE SET clean_title = excluded.clean_title, year = excluded.year",
        params![fields.clean_title, fields.year, fields.match_key, now_epoch()],
    )?;
    let series_id: i64 =
        conn.query_row("SELECT id FROM series WHERE match_key = ?1", params![fields.match_key], |row| row.get(0))?;

    conn.execute(
        "INSERT INTO series_sources (series_id, provider_id, remote_id, raw_title)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            series_id = excluded.series_id,
            raw_title = excluded.raw_title",
        params![series_id, provider_id, series.remote_id, series.raw_title],
    )?;
    Ok(())
}

/// Borra `movies`/`series` sin ninguna fuente asociada. Recalcular `match_key` con
/// normalización real (F3) puede dejar huérfanas las filas placeholder de F2 al
/// resincronizar: el `ON CONFLICT(match_key)` ya no coincide con la fila vieja.
pub fn delete_orphaned_movies_and_series(conn: &Connection) -> Result<(usize, usize)> {
    let movies = conn.execute("DELETE FROM movies WHERE id NOT IN (SELECT DISTINCT movie_id FROM movie_sources)", [])?;
    let series = conn.execute("DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM series_sources)", [])?;
    Ok((movies, series))
}

/// Reconstruye `search_index` (FTS5) entero de una vez al final de cada sincronización.
/// Se prefirió a triggers por fila: con decenas de miles de upserts por sincronización,
/// un trigger de borrado por fila sobre una tabla FTS5 sin índice en `item_id` sería
/// O(n²) en la práctica. Reconstruir de una vez es un único escaneo por tabla.
pub fn rebuild_search_index(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM search_index", [])?;
    conn.execute(
        "INSERT INTO search_index(title, item_kind, item_id) SELECT display_name, 'channel', id FROM channels",
        [],
    )?;
    conn.execute(
        "INSERT INTO search_index(title, item_kind, item_id) SELECT clean_title, 'movie', id FROM movies",
        [],
    )?;
    conn.execute(
        "INSERT INTO search_index(title, item_kind, item_id) SELECT clean_title, 'series', id FROM series",
        [],
    )?;
    Ok(())
}

pub fn count_categories(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM categories WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

pub fn count_channels(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM channels WHERE provider_id = ?1", params![provider_id], |row| row.get(0))
        .map_err(Into::into)
}

pub fn count_movie_sources(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM movie_sources WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

pub fn count_series_sources(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM series_sources WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

pub fn count_matched_movies(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM movies WHERE tmdb_state = 'matched'", [], |row| row.get(0)).map_err(Into::into)
}

pub fn count_all_movies(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM movies", [], |row| row.get(0)).map_err(Into::into)
}

// ─────────────── TMDB (F3) ───────────────

pub struct PendingTitle {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub tmdb_id: Option<i64>,
}

pub fn list_pending_movies(conn: &Connection) -> Result<Vec<PendingTitle>> {
    list_pending(conn, "movies")
}

pub fn list_pending_series(conn: &Connection) -> Result<Vec<PendingTitle>> {
    list_pending(conn, "series")
}

fn list_pending(conn: &Connection, table: &str) -> Result<Vec<PendingTitle>> {
    // `table` no viene nunca del usuario, solo de las dos llamadas de arriba.
    // 'error' se reintenta también: es un fallo transitorio (red, 429 agotado), no un
    // "TMDB no tiene esto" definitivo como sí lo es 'not_found'.
    let sql = format!("SELECT id, clean_title, year, tmdb_id FROM {table} WHERE tmdb_state IN ('pending', 'error')");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(PendingTitle { id: row.get(0)?, clean_title: row.get(1)?, year: row.get(2)?, tmdb_id: row.get(3)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Resultado de intentar emparejar una película/serie con TMDB. `genres_json` es el
/// array de ids de género tal cual lo da TMDB, ya serializado.
pub struct TmdbOutcome<'a> {
    pub state: &'a str, // 'matched' | 'not_found' | 'error'
    pub tmdb_id: Option<i64>,
    pub overview: Option<&'a str>,
    pub poster_path: Option<&'a str>,
    pub backdrop_path: Option<&'a str>,
    pub rating: Option<f64>,
    pub genres_json: Option<&'a str>,
    pub runtime_min: Option<i32>,
}

pub fn update_movie_tmdb_result(conn: &Connection, movie_id: i64, outcome: &TmdbOutcome) -> Result<()> {
    conn.execute(
        "UPDATE movies SET tmdb_state=?1, tmdb_id=?2, overview=?3, poster_path=?4, backdrop_path=?5, rating=?6,
            genres=?7, runtime_min=?8 WHERE id=?9",
        params![
            outcome.state,
            outcome.tmdb_id,
            outcome.overview,
            outcome.poster_path,
            outcome.backdrop_path,
            outcome.rating,
            outcome.genres_json,
            outcome.runtime_min,
            movie_id
        ],
    )?;
    Ok(())
}

pub fn update_series_tmdb_result(conn: &Connection, series_id: i64, outcome: &TmdbOutcome) -> Result<()> {
    conn.execute(
        "UPDATE series SET tmdb_state=?1, tmdb_id=?2, overview=?3, poster_path=?4, backdrop_path=?5, rating=?6,
            genres=?7 WHERE id=?8",
        params![
            outcome.state,
            outcome.tmdb_id,
            outcome.overview,
            outcome.poster_path,
            outcome.backdrop_path,
            outcome.rating,
            outcome.genres_json,
            series_id
        ],
    )?;
    Ok(())
}

/// Caché de aciertos y de fallos de búsqueda en TMDB (regla: "los fallos también se
/// cachean", ver `docs/DATA-MODEL.md`).
pub fn get_tmdb_cache(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT response FROM tmdb_cache WHERE query_hash = ?1", params![key], |row| row.get(0))
        .optional()
        .map_err(Into::into)
}

pub fn set_tmdb_cache(conn: &Connection, key: &str, response_json: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO tmdb_cache (query_hash, response, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(query_hash) DO UPDATE SET response = excluded.response, fetched_at = excluded.fetched_at",
        params![key, response_json, now_epoch()],
    )?;
    Ok(())
}

// ─────────────── Caché de inspección de códecs (F6) ───────────────

pub struct ProbeCacheRow {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    pub play_tier: i64,
    /// Duración real en segundos (VOD; sin sentido en directo). Ver
    /// `media::probe::ProbeResult::duration_seconds`.
    pub duration_s: Option<f64>,
}

/// `stream_key` es `media::probe::stream_cache_key(url)`: un hash, nunca la URL en sí
/// (regla R5). Ver `docs/ARCHITECTURE.md`, "ffprobe inspecciona el stream una vez y el
/// resultado se cachea por ítem".
pub fn get_probe_cache(conn: &Connection, stream_key: &str) -> Result<Option<ProbeCacheRow>> {
    conn.query_row(
        "SELECT video_codec, audio_codec, container, play_tier, duration_s FROM probe_cache WHERE stream_key = ?1",
        params![stream_key],
        |row| {
            Ok(ProbeCacheRow {
                video_codec: row.get(0)?,
                audio_codec: row.get(1)?,
                container: row.get(2)?,
                play_tier: row.get(3)?,
                duration_s: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub fn set_probe_cache(
    conn: &Connection,
    stream_key: &str,
    video_codec: Option<&str>,
    audio_codec: Option<&str>,
    container: Option<&str>,
    play_tier: i64,
    duration_s: Option<f64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO probe_cache (stream_key, video_codec, audio_codec, container, play_tier, duration_s, probed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(stream_key) DO UPDATE SET
            video_codec = excluded.video_codec, audio_codec = excluded.audio_codec,
            container = excluded.container, play_tier = excluded.play_tier,
            duration_s = excluded.duration_s, probed_at = excluded.probed_at",
        params![stream_key, video_codec, audio_codec, container, play_tier, duration_s, now_epoch()],
    )?;
    Ok(())
}

// ─────────────── EPG (F7) ───────────────
//
// Tipo local, no `catalog::epg::Programme`: `queries.rs` no depende de `catalog` (regla
// de capas del proyecto — lógica de negocio en `catalog`, SQL aquí), igual que
// `TitleFields` ya hace para el sincronizador de catálogo.

pub struct EpgProgrammeRow<'a> {
    pub channel_id: &'a str,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    pub description: Option<&'a str>,
    pub category: Option<&'a str>,
    pub icon_url: Option<&'a str>,
}

/// La guía se reemplaza entera en cada sincronización (como `search_index`, ver
/// `rebuild_search_index`): más simple y barato que reconciliar fila a fila, y XMLTV no
/// da un identificador estable de programa con el que hacer upsert de todas formas.
pub fn delete_all_epg_programmes(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM epg_programmes", [])?;
    Ok(())
}

pub fn insert_epg_programmes_batch(conn: &Connection, batch: &[EpgProgrammeRow]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title, subtitle, description, category, icon_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for p in batch {
        stmt.execute(params![p.channel_id, p.start_ts, p.stop_ts, p.title, p.subtitle, p.description, p.category, p.icon_url])?;
    }
    Ok(())
}

/// Cuántos canales con `epg_channel_id` no tienen ni un solo programa en la guía
/// importada — el informe que pide `docs/PROMPTS.md` ("cuántos canales quedan sin EPG").
pub fn count_channels_without_epg(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM channels
         WHERE epg_channel_id IS NOT NULL
           AND epg_channel_id NOT IN (SELECT DISTINCT channel_id FROM epg_programmes)",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Registra qué nivel de la cascada sirvió un ítem, para el diagnóstico que menciona
/// `docs/ARCHITECTURE.md` ("si el nivel 3 aparece a menudo, hay que revisar la
/// detección de códecs"). No toca `position_s` si la fila ya existía: el punto de
/// reproducción es responsabilidad exclusiva de `upsert_playback_progress`.
pub fn record_play_tier(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64, play_tier: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, duration_s, finished, played_at, play_tier)
         VALUES (?1, ?2, ?3, 0, NULL, 0, ?4, ?5)
         ON CONFLICT(profile_id, item_kind, item_id) DO UPDATE SET play_tier = excluded.play_tier, played_at = excluded.played_at",
        params![profile_id, item_kind, item_id, now_epoch(), play_tier],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_connection_at;
    use std::path::Path;

    fn memory_conn() -> Connection {
        open_connection_at(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn insert_and_list_provider_roundtrip() {
        let conn = memory_conn();
        let id = insert_provider(&conn, ProviderKind::Xtream, "Mi proveedor", "http://panel.example.com:8080", Some("demo"), 2, Some(1893456000)).unwrap();

        let providers = list_providers(&conn).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, id);
        assert_eq!(providers[0].kind, ProviderKind::Xtream);
        assert_eq!(providers[0].max_connections, 2);
        assert!(providers[0].credential_ref.is_none());

        set_provider_credential_ref(&conn, id, "provider-1").unwrap();
        let fetched = get_provider(&conn, id).unwrap().unwrap();
        assert_eq!(fetched.credential_ref.as_deref(), Some("provider-1"));
    }

    #[test]
    fn upsert_categories_returns_remote_id_map_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let cats = vec![
            RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() },
            RawCategory { remote_id: "2".into(), kind: MediaKind::Live, name: "DOCUS".into() },
        ];
        let map1 = upsert_categories(&conn, provider_id, MediaKind::Live, &cats).unwrap();
        assert_eq!(map1.len(), 2);

        // Repetir con el mismo remote_id pero nombre distinto: actualiza, no duplica.
        let cats2 = vec![RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA HD".into() }];
        let map2 = upsert_categories(&conn, provider_id, MediaKind::Live, &cats2).unwrap();
        assert_eq!(map2["1"], map1["1"]);

        let total = count_categories(&conn, provider_id).unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn upsert_channel_links_category_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let cats = vec![RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() }];
        let category_map = upsert_categories(&conn, provider_id, MediaKind::Live, &cats).unwrap();

        let channel = RawChannel {
            remote_id: "101".into(),
            category_remote_id: Some("1".into()),
            raw_name: "Canal Uno".into(),
            logo_url: None,
            epg_channel_id: None,
            has_catchup: true,
            catchup_days: 7,
            sort_order: Some(1),
        };
        let fields = TitleFields { clean_title: "Canal Uno", year: None, match_key: "n/a", quality: None, language: None };
        upsert_channel(&conn, provider_id, &category_map, &channel, &fields).unwrap();
        upsert_channel(&conn, provider_id, &category_map, &channel, &fields).unwrap(); // repetir: no duplica

        assert_eq!(count_channels(&conn, provider_id).unwrap(), 1);
    }

    #[test]
    fn upsert_movie_groups_by_match_key() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let category_map = HashMap::new();

        let movie_a = RawMovie {
            remote_id: "5001".into(),
            category_remote_id: None,
            raw_title: "Mi Película 4K".into(),
            container_extension: Some("mkv".into()),
            logo_url: None,
            added_at: Some(1700000000),
            rating: Some(7.5),
            tmdb_id: None,
        };
        // Misma película, otro proveedor/remote_id, distinta calidad en el nombre crudo,
        // pero el mismo match_key normalizado: debe agruparse bajo la misma fila `movies`.
        let movie_b = RawMovie { remote_id: "9999".into(), raw_title: "Mi Película HD".into(), ..movie_a.clone() };

        let fields_a =
            TitleFields { clean_title: "Mi Película", year: Some(2020), match_key: "mi-pelicula-2020", quality: Some("4K"), language: None };
        let fields_b = TitleFields { quality: Some("HD"), ..fields_a };

        upsert_movie(&conn, provider_id, &category_map, &movie_a, &fields_a).unwrap();
        upsert_movie(&conn, provider_id, &category_map, &movie_b, &fields_b).unwrap();

        assert_eq!(count_all_movies(&conn).unwrap(), 1);
        assert_eq!(count_movie_sources(&conn, provider_id).unwrap(), 2);
    }

    #[test]
    fn upsert_series_has_no_category_column_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let series = RawSeries {
            remote_id: "9001".into(),
            raw_title: "Serie de Ejemplo".into(),
            cover_url: None,
            plot: None,
            release_date: None,
        };
        let fields = TitleFields { clean_title: "Serie de Ejemplo", year: None, match_key: "serie-de-ejemplo-0000", quality: None, language: None };
        upsert_series(&conn, provider_id, &series, &fields).unwrap();
        upsert_series(&conn, provider_id, &series, &fields).unwrap();

        assert_eq!(count_series_sources(&conn, provider_id).unwrap(), 1);
    }

    #[test]
    fn recomputing_match_key_orphans_the_old_row_and_cleanup_removes_it() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let category_map = HashMap::new();
        let movie = RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "Peli".into(),
            container_extension: None,
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: None,
        };

        let placeholder = TitleFields { clean_title: "Peli", year: None, match_key: "peli-0000-placeholder", quality: None, language: None };
        upsert_movie(&conn, provider_id, &category_map, &movie, &placeholder).unwrap();
        assert_eq!(count_all_movies(&conn).unwrap(), 1);

        // Resincronizar con un match_key distinto (normalización real): el movie_source
        // se mueve a una fila nueva, la vieja queda huérfana.
        let real = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-2020", quality: None, language: None };
        upsert_movie(&conn, provider_id, &category_map, &movie, &real).unwrap();
        assert_eq!(count_all_movies(&conn).unwrap(), 2); // la vieja sigue ahí, huérfana

        let (removed_movies, removed_series) = delete_orphaned_movies_and_series(&conn).unwrap();
        assert_eq!(removed_movies, 1);
        assert_eq!(removed_series, 0);
        assert_eq!(count_all_movies(&conn).unwrap(), 1);
        assert_eq!(count_movie_sources(&conn, provider_id).unwrap(), 1);
    }

    #[test]
    fn tmdb_cache_roundtrip() {
        let conn = memory_conn();
        assert_eq!(get_tmdb_cache(&conn, "movie:foo:2020").unwrap(), None);

        set_tmdb_cache(&conn, "movie:foo:2020", r#"{"outcome":"not_found"}"#).unwrap();
        assert_eq!(get_tmdb_cache(&conn, "movie:foo:2020").unwrap().as_deref(), Some(r#"{"outcome":"not_found"}"#));

        set_tmdb_cache(&conn, "movie:foo:2020", r#"{"outcome":"matched","id":1}"#).unwrap();
        assert_eq!(get_tmdb_cache(&conn, "movie:foo:2020").unwrap().as_deref(), Some(r#"{"outcome":"matched","id":1}"#));
    }

    #[test]
    fn list_pending_movies_and_update_result() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie = RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "Peli".into(),
            container_extension: None,
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: None,
        };
        let fields = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-2020", quality: None, language: None };
        upsert_movie(&conn, provider_id, &HashMap::new(), &movie, &fields).unwrap();

        let pending = list_pending_movies(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].clean_title, "Peli");

        let outcome = TmdbOutcome {
            state: "matched",
            tmdb_id: Some(603692),
            overview: Some("Sinopsis"),
            poster_path: Some("/p.jpg"),
            backdrop_path: Some("/b.jpg"),
            rating: Some(7.8),
            genres_json: Some("[28,80]"),
            runtime_min: Some(118),
        };
        update_movie_tmdb_result(&conn, pending[0].id, &outcome).unwrap();

        assert!(list_pending_movies(&conn).unwrap().is_empty());
        assert_eq!(count_matched_movies(&conn).unwrap(), 1);
    }

    #[test]
    fn epg_batch_insert_and_channel_coverage_report() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let channel_with_epg = RawChannel {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_name: "Canal Uno".into(),
            logo_url: None,
            epg_channel_id: Some("canaluno.es".into()),
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let channel_without_epg = RawChannel { remote_id: "2".into(), epg_channel_id: Some("singuia.es".into()), ..channel_with_epg.clone() };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        upsert_channel(&conn, provider_id, &HashMap::new(), &channel_with_epg, &fields).unwrap();
        upsert_channel(&conn, provider_id, &HashMap::new(), &channel_without_epg, &fields).unwrap();

        assert_eq!(count_channels_without_epg(&conn).unwrap(), 2); // todavía no hay guía importada.

        let batch = vec![EpgProgrammeRow {
            channel_id: "canaluno.es",
            start_ts: 1_700_000_000,
            stop_ts: 1_700_003_600,
            title: "Telediario",
            subtitle: None,
            description: None,
            category: None,
            icon_url: None,
        }];
        insert_epg_programmes_batch(&conn, &batch).unwrap();

        assert_eq!(count_channels_without_epg(&conn).unwrap(), 1, "canaluno.es ya tiene guía, singuia.es sigue sin ella");

        delete_all_epg_programmes(&conn).unwrap();
        assert_eq!(count_channels_without_epg(&conn).unwrap(), 2);
    }

    #[test]
    fn probe_cache_roundtrip_and_update() {
        let conn = memory_conn();
        assert!(get_probe_cache(&conn, "probe:abc").unwrap().is_none());

        set_probe_cache(&conn, "probe:abc", Some("h264"), Some("aac"), Some("mpegts"), 2, Some(1425.0)).unwrap();
        let cached = get_probe_cache(&conn, "probe:abc").unwrap().unwrap();
        assert_eq!(cached.video_codec.as_deref(), Some("h264"));
        assert_eq!(cached.play_tier, 2);
        assert_eq!(cached.duration_s, Some(1425.0));

        // Reprobar el mismo stream (p. ej. el proveedor cambió de códec) actualiza, no duplica.
        set_probe_cache(&conn, "probe:abc", Some("hevc"), Some("aac"), Some("mpegts"), 3, None).unwrap();
        let updated = get_probe_cache(&conn, "probe:abc").unwrap().unwrap();
        assert_eq!(updated.video_codec.as_deref(), Some("hevc"));
        assert_eq!(updated.play_tier, 3);
    }

    #[test]
    fn record_play_tier_does_not_disturb_existing_position() {
        let conn = memory_conn();
        let profile_id = crate::db::reads::ensure_default_profile(&conn).unwrap();
        crate::db::reads::upsert_playback_progress(&conn, profile_id, "channel", 1, 0, None).unwrap();
        record_play_tier(&conn, profile_id, "channel", 1, 2).unwrap();

        let tier: i64 = conn
            .query_row("SELECT play_tier FROM playback_state WHERE profile_id=?1 AND item_id=1", params![profile_id], |r| r.get(0))
            .unwrap();
        assert_eq!(tier, 2);

        // Un ítem nunca reproducido (sin progreso previo) también debe poder recibir tier.
        record_play_tier(&conn, profile_id, "movie", 5, 1).unwrap();
        let tier2: i64 = conn
            .query_row("SELECT play_tier FROM playback_state WHERE profile_id=?1 AND item_id=5", params![profile_id], |r| r.get(0))
            .unwrap();
        assert_eq!(tier2, 1);
    }
}
