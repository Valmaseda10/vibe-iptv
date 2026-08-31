//! Consultas de lectura para la API HTTP de F4. Separadas de `queries.rs` (que cubre la
//! escritura del sincronizador) por claridad; siguen siendo el único sitio con SQL para
//! estos datos.
//!
//! Paginación por cursor (`after` = último `id` visto, nunca offset, regla R4). Con
//! filtros combinados con un orden distinto al de inserción, el cursor se aproxima con
//! `id` como desempate: es un límite conocido y documentado, no un descuido — hacer
//! keyset pagination exacto por cada combinación de orden/filtro no compensaba el tiempo
//! en esta fase, sin una interfaz real todavía que lo ejercite (llega en F5).

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, ToSql};

use crate::util::redact;

use super::{now_epoch, Result};

pub struct CategoryRow {
    pub id: i64,
    pub name: String,
}

/// `include_adult` en `false` oculta las categorías marcadas `is_adult` (control
/// parental, F8): mientras no se desbloquee con el PIN, ni siquiera aparecen en el
/// selector de categorías.
pub fn list_categories(conn: &Connection, kind: &str, include_adult: bool) -> Result<Vec<CategoryRow>> {
    let sql = if include_adult {
        "SELECT id, name FROM categories WHERE kind = ?1 ORDER BY name"
    } else {
        "SELECT id, name FROM categories WHERE kind = ?1 AND is_adult = 0 ORDER BY name"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![kind], |row| Ok(CategoryRow { id: row.get(0)?, name: row.get(1)? }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct ChannelListItem {
    pub id: i64,
    pub display_name: String,
    pub logo_url: Option<String>,
    pub category_id: Option<i64>,
    pub has_catchup: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn list_channels_page(
    conn: &Connection,
    category_id: Option<i64>,
    search: Option<&str>,
    after: Option<i64>,
    limit: i64,
    include_adult: bool,
) -> Result<Vec<ChannelListItem>> {
    let mut sql = String::from(
        "SELECT id, display_name, logo_url, category_id, has_catchup FROM channels WHERE 1=1",
    );
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if !include_adult {
        sql.push_str(" AND (category_id IS NULL OR category_id NOT IN (SELECT id FROM categories WHERE is_adult = 1))");
    }
    if let Some(cat) = category_id {
        sql.push_str(" AND category_id = ?");
        args.push(Box::new(cat));
    }
    if let Some(q) = search {
        sql.push_str(" AND display_name LIKE ? ESCAPE '\\'");
        args.push(Box::new(like_pattern(q)));
    }
    if let Some(a) = after {
        sql.push_str(" AND id > ?");
        args.push(Box::new(a));
    }
    sql.push_str(" ORDER BY sort_order IS NULL, sort_order, id LIMIT ?");
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        // logo_url del proveedor se redacta antes de salir de la capa de datos: la API
        // HTTP no debe filtrar URLs de proveedor (regla R5), aunque un logo en concreto
        // no suela llevar credenciales.
        let raw_logo: Option<String> = row.get(2)?;
        Ok(ChannelListItem {
            id: row.get(0)?,
            display_name: row.get(1)?,
            logo_url: raw_logo.map(|url| redact(&url)),
            category_id: row.get(3)?,
            has_catchup: row.get::<_, i64>(4)? != 0,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovieSort {
    AddedDesc,
    TitleAsc,
    YearDesc,
    RatingDesc,
}

pub struct MovieListItem {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub tmdb_state: String,
}

#[allow(clippy::too_many_arguments)]
pub fn list_movies_page(
    conn: &Connection,
    genre: Option<i64>,
    year: Option<i32>,
    quality: Option<&str>,
    sort: MovieSort,
    after: Option<i64>,
    limit: i64,
    include_adult: bool,
) -> Result<Vec<MovieListItem>> {
    let mut sql = String::from("SELECT DISTINCT m.id, m.clean_title, m.year, m.poster_path, m.rating, m.tmdb_state FROM movies m");
    if quality.is_some() {
        sql.push_str(" JOIN movie_sources ms ON ms.movie_id = m.id");
    }
    sql.push_str(" WHERE 1=1");

    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if !include_adult {
        // Se oculta si TODAS sus fuentes son de categorías adultas (control parental,
        // F8) — más precisamente, se excluye si tiene alguna fuente en una categoría
        // adulta: mejor ocultar de más que enseñar de menos en un filtro parental.
        sql.push_str(
            " AND NOT EXISTS (SELECT 1 FROM movie_sources ms2 JOIN categories c ON c.id = ms2.category_id
                               WHERE ms2.movie_id = m.id AND c.is_adult = 1)",
        );
    }
    if let Some(g) = genre {
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(m.genres) WHERE value = ?)");
        args.push(Box::new(g));
    }
    if let Some(y) = year {
        sql.push_str(" AND m.year = ?");
        args.push(Box::new(y));
    }
    if let Some(q) = quality {
        sql.push_str(" AND ms.quality = ?");
        args.push(Box::new(q.to_string()));
    }
    let (cursor_op, order) = match sort {
        MovieSort::AddedDesc => ("<", "m.id DESC"),
        MovieSort::TitleAsc => (">", "m.clean_title ASC, m.id ASC"),
        MovieSort::YearDesc => ("<", "m.year DESC, m.id DESC"),
        MovieSort::RatingDesc => ("<", "m.rating DESC, m.id DESC"),
    };
    if let Some(a) = after {
        sql.push_str(&format!(" AND m.id {cursor_op} ?"));
        args.push(Box::new(a));
    }
    sql.push_str(&format!(" ORDER BY {order} LIMIT ?"));
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        Ok(MovieListItem {
            id: row.get(0)?,
            clean_title: row.get(1)?,
            year: row.get(2)?,
            poster_path: row.get(3)?,
            rating: row.get(4)?,
            tmdb_state: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct MovieSourceItem {
    pub id: i64,
    pub provider_id: i64,
    pub remote_id: String,
    pub quality: Option<String>,
    pub language: Option<String>,
    pub container: Option<String>,
}

pub struct MovieDetail {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    pub runtime_min: Option<i32>,
    pub genres: Vec<i64>,
    pub tmdb_state: String,
    pub sources: Vec<MovieSourceItem>,
}

pub fn get_movie(conn: &Connection, id: i64) -> Result<Option<MovieDetail>> {
    let base = conn
        .query_row(
            "SELECT id, clean_title, year, overview, poster_path, backdrop_path, rating, runtime_min, genres, tmdb_state
             FROM movies WHERE id = ?1",
            params![id],
            |row| {
                let genres_json: Option<String> = row.get(8)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i32>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<f64>>(6)?,
                    row.get::<_, Option<i32>>(7)?,
                    genres_json,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?;

    let Some((id, clean_title, year, overview, poster_path, backdrop_path, rating, runtime_min, genres_json, tmdb_state)) = base else {
        return Ok(None);
    };

    let genres: Vec<i64> = genres_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();

    let mut stmt = conn.prepare(
        "SELECT id, provider_id, remote_id, quality, language, container FROM movie_sources WHERE movie_id = ?1",
    )?;
    let sources = stmt
        .query_map(params![id], |row| {
            Ok(MovieSourceItem {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                remote_id: row.get(2)?,
                quality: row.get(3)?,
                language: row.get(4)?,
                container: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(MovieDetail { id, clean_title, year, overview, poster_path, backdrop_path, rating, runtime_min, genres, tmdb_state, sources }))
}

pub struct SeriesListItem {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub tmdb_state: String,
}

pub fn list_series_page(
    conn: &Connection,
    genre: Option<i64>,
    year: Option<i32>,
    after: Option<i64>,
    limit: i64,
) -> Result<Vec<SeriesListItem>> {
    let mut sql = String::from("SELECT id, clean_title, year, poster_path, rating, tmdb_state FROM series WHERE 1=1");
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(g) = genre {
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(genres) WHERE value = ?)");
        args.push(Box::new(g));
    }
    if let Some(y) = year {
        sql.push_str(" AND year = ?");
        args.push(Box::new(y));
    }
    if let Some(a) = after {
        sql.push_str(" AND id < ?");
        args.push(Box::new(a));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        Ok(SeriesListItem { id: row.get(0)?, clean_title: row.get(1)?, year: row.get(2)?, poster_path: row.get(3)?, rating: row.get(4)?, tmdb_state: row.get(5)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct SeriesSourceItem {
    pub id: i64,
    pub provider_id: i64,
    pub remote_id: String,
    pub detail_synced_at: Option<i64>,
}

pub struct EpisodeItem {
    pub id: i64,
    pub series_source_id: i64,
    pub remote_id: String,
    pub season: i64,
    pub number: i64,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub duration_s: Option<i64>,
}

pub struct SeriesDetail {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    pub genres: Vec<i64>,
    pub tmdb_state: String,
    pub sources: Vec<SeriesSourceItem>,
    pub episodes: Vec<EpisodeItem>,
}

pub fn get_series(conn: &Connection, id: i64) -> Result<Option<SeriesDetail>> {
    let base = conn
        .query_row(
            "SELECT id, clean_title, year, overview, poster_path, backdrop_path, rating, genres, tmdb_state FROM series WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i32>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<f64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?;

    let Some((id, clean_title, year, overview, poster_path, backdrop_path, rating, genres_json, tmdb_state)) = base else {
        return Ok(None);
    };
    let genres: Vec<i64> = genres_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();

    let mut source_stmt = conn.prepare("SELECT id, provider_id, remote_id, detail_synced_at FROM series_sources WHERE series_id = ?1")?;
    let sources = source_stmt
        .query_map(params![id], |row| {
            Ok(SeriesSourceItem { id: row.get(0)?, provider_id: row.get(1)?, remote_id: row.get(2)?, detail_synced_at: row.get(3)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut episode_stmt = conn.prepare(
        "SELECT e.id, e.series_source_id, e.remote_id, e.season, e.number, e.title, e.overview, e.still_path, e.duration_s
         FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id
         WHERE ss.series_id = ?1 ORDER BY e.season, e.number",
    )?;
    let episodes = episode_stmt
        .query_map(params![id], |row| {
            Ok(EpisodeItem {
                id: row.get(0)?,
                series_source_id: row.get(1)?,
                remote_id: row.get(2)?,
                season: row.get(3)?,
                number: row.get(4)?,
                title: row.get(5)?,
                overview: row.get(6)?,
                still_path: row.get(7)?,
                duration_s: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(SeriesDetail { id, clean_title, year, overview, poster_path, backdrop_path, rating, genres, tmdb_state, sources, episodes }))
}

/// `series_source_id` es de la primera fuente que tenga esta serie; se usa cuando hace
/// falta pedir `series_detail()` bajo demanda (ver `docs/XTREAM-API.md`).
pub fn oldest_series_source_needing_detail(conn: &Connection, series_id: i64, staleness_seconds: i64) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT id, provider_id FROM series_sources
         WHERE series_id = ?1 AND (detail_synced_at IS NULL OR detail_synced_at < ?2)
         ORDER BY id LIMIT 1",
        params![series_id, now_epoch() - staleness_seconds],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

pub fn upsert_episodes(conn: &Connection, series_source_id: i64, episodes: &[crate::sources::model::RawEpisode]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO episodes (series_source_id, remote_id, season, number, title, still_path, duration_s, container)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(series_source_id, remote_id) DO UPDATE SET
            season = excluded.season, number = excluded.number, title = excluded.title,
            still_path = excluded.still_path, duration_s = excluded.duration_s, container = excluded.container",
    )?;
    for ep in episodes {
        stmt.execute(params![series_source_id, ep.remote_id, ep.season, ep.number, ep.title, ep.still_url, ep.duration_s, ep.container_extension])?;
    }
    Ok(())
}

pub fn mark_series_source_detail_synced(conn: &Connection, series_source_id: i64) -> Result<()> {
    conn.execute("UPDATE series_sources SET detail_synced_at = ?1 WHERE id = ?2", params![now_epoch(), series_source_id])?;
    Ok(())
}

pub struct SearchHit {
    pub item_kind: String,
    pub item_id: i64,
    pub title: String,
}

pub fn search(conn: &Connection, query: &str, limit_per_kind: i64) -> Result<Vec<SearchHit>> {
    let mut stmt = conn.prepare(
        "SELECT item_kind, item_id, title FROM search_index WHERE search_index MATCH ?1
         ORDER BY rank LIMIT ?2",
    )?;
    // FTS5 necesita una consulta bien formada; una entrada vacía o con solo espacios no
    // debe reventar, solo no encontrar nada.
    let fts_query = format!("{}*", fts_escape(query));
    let rows = stmt
        .query_map(params![fts_query, limit_per_kind * 3], |row| {
            Ok(SearchHit { item_kind: row.get(0)?, item_id: row.get(1)?, title: row.get(2)? })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>());

    match rows {
        Ok(hits) => Ok(hits),
        Err(_) => Ok(Vec::new()), // consulta FTS5 mal formada (p. ej. solo símbolos): sin resultados, no error 500
    }
}

fn fts_escape(query: &str) -> String {
    let cleaned: String = query.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    format!("\"{}\"", cleaned.trim())
}

fn like_pattern(input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

// ─────────────── Resolución de streams (F6) ───────────────
//
// El proxy de streaming (`server::stream`) necesita provider_id + remote_id (+
// container, en VOD) para reconstruir la URL real con `Source::stream_url`. Ninguna de
// estas funciones expone esa URL: solo los datos con los que construirla, que quedan
// dentro del núcleo (regla R5 — la API nunca devuelve URLs de proveedor).

pub struct ChannelStreamInfo {
    pub provider_id: i64,
    pub remote_id: String,
}

pub fn resolve_channel_stream(conn: &Connection, channel_id: i64) -> Result<Option<ChannelStreamInfo>> {
    conn.query_row(
        "SELECT provider_id, remote_id FROM channels WHERE id = ?1",
        params![channel_id],
        |row| Ok(ChannelStreamInfo { provider_id: row.get(0)?, remote_id: row.get(1)? }),
    )
    .optional()
    .map_err(Into::into)
}

/// Datos para decidir y construir una petición de catch-up (F9): además de lo que
/// necesita `resolve_channel_stream`, cuántos días atrás guarda el proveedor. El
/// llamador compara eso contra el `start` pedido antes de construir ninguna URL.
pub struct ChannelCatchupInfo {
    pub provider_id: i64,
    pub remote_id: String,
    pub has_catchup: bool,
    pub catchup_days: u32,
}

pub fn resolve_channel_catchup(conn: &Connection, channel_id: i64) -> Result<Option<ChannelCatchupInfo>> {
    conn.query_row(
        "SELECT provider_id, remote_id, has_catchup, catchup_days FROM channels WHERE id = ?1",
        params![channel_id],
        |row| {
            Ok(ChannelCatchupInfo {
                provider_id: row.get(0)?,
                remote_id: row.get(1)?,
                has_catchup: row.get::<_, i64>(2)? != 0,
                catchup_days: row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u32,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub struct MovieStreamInfo {
    pub provider_id: i64,
    pub remote_id: String,
    pub container_extension: Option<String>,
}

/// Si `source_id` se da, resuelve esa fuente concreta (varias calidades de la misma
/// película); si no, la primera disponible.
pub fn resolve_movie_stream(conn: &Connection, movie_id: i64, source_id: Option<i64>) -> Result<Option<MovieStreamInfo>> {
    let row = if let Some(source_id) = source_id {
        conn.query_row(
            "SELECT provider_id, remote_id, container FROM movie_sources WHERE id = ?1 AND movie_id = ?2",
            params![source_id, movie_id],
            |row| Ok(MovieStreamInfo { provider_id: row.get(0)?, remote_id: row.get(1)?, container_extension: row.get(2)? }),
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT provider_id, remote_id, container FROM movie_sources WHERE movie_id = ?1 ORDER BY id LIMIT 1",
            params![movie_id],
            |row| Ok(MovieStreamInfo { provider_id: row.get(0)?, remote_id: row.get(1)?, container_extension: row.get(2)? }),
        )
        .optional()?
    };
    Ok(row)
}

pub struct EpisodeStreamInfo {
    pub provider_id: i64,
    pub remote_id: String,
    pub container_extension: Option<String>,
}

pub fn resolve_episode_stream(conn: &Connection, episode_id: i64) -> Result<Option<EpisodeStreamInfo>> {
    conn.query_row(
        "SELECT ss.provider_id, e.remote_id, e.container
         FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id
         WHERE e.id = ?1",
        params![episode_id],
        |row| Ok(EpisodeStreamInfo { provider_id: row.get(0)?, remote_id: row.get(1)?, container_extension: row.get(2)? }),
    )
    .optional()
    .map_err(Into::into)
}

// ─────────────── Guía EPG (F7) ───────────────
//
// `epg_programmes.channel_id` es el id de XMLTV (`channels.epg_channel_id`), no el id
// interno con el que trabaja el resto de la API: estas funciones reciben y devuelven
// siempre el id interno, y hacen la traducción por dentro — el resto de la app nunca
// necesita saber que existe un id de XMLTV.

pub struct ProgrammeSummary {
    pub title: String,
    pub subtitle: Option<String>,
    pub start_ts: i64,
    pub stop_ts: i64,
}

pub struct NowNextProgramme {
    pub channel_id: i64,
    pub current: Option<ProgrammeSummary>,
    pub next: Option<ProgrammeSummary>,
}

fn programme_row(row: &Row) -> rusqlite::Result<ProgrammeSummary> {
    Ok(ProgrammeSummary { title: row.get(0)?, subtitle: row.get(1)?, start_ts: row.get(2)?, stop_ts: row.get(3)? })
}

/// Programa actual y siguiente de cada canal en `channel_ids` (ids internos). Un canal
/// sin `epg_channel_id`, o sin guía importada todavía, sale con `current`/`next` a
/// `None` — nunca se descarta de la lista, para que el llamador sepa que se pidió.
pub fn epg_now_and_next(conn: &Connection, channel_ids: &[i64], now: i64) -> Result<Vec<NowNextProgramme>> {
    let mut results = Vec::with_capacity(channel_ids.len());
    for &channel_id in channel_ids {
        let epg_channel_id: Option<String> = conn
            .query_row("SELECT epg_channel_id FROM channels WHERE id = ?1", params![channel_id], |row| row.get(0))
            .optional()?
            .flatten();

        let Some(epg_channel_id) = epg_channel_id else {
            results.push(NowNextProgramme { channel_id, current: None, next: None });
            continue;
        };

        let current = conn
            .query_row(
                "SELECT title, subtitle, start_ts, stop_ts FROM epg_programmes
                 WHERE channel_id = ?1 AND start_ts <= ?2 AND stop_ts > ?2 ORDER BY start_ts LIMIT 1",
                params![epg_channel_id, now],
                programme_row,
            )
            .optional()?;
        let next = conn
            .query_row(
                "SELECT title, subtitle, start_ts, stop_ts FROM epg_programmes
                 WHERE channel_id = ?1 AND start_ts > ?2 ORDER BY start_ts LIMIT 1",
                params![epg_channel_id, now],
                programme_row,
            )
            .optional()?;

        results.push(NowNextProgramme { channel_id, current, next });
    }
    Ok(results)
}

pub struct GridProgramme {
    pub channel_id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub category: Option<String>,
}

/// Todos los programas de `channel_ids` que se solapan con `[from_ts, to_ts)` — la vista
/// de guía en rejilla. Ordenado por canal y luego por hora de inicio.
pub fn epg_grid(conn: &Connection, channel_ids: &[i64], from_ts: i64, to_ts: i64) -> Result<Vec<GridProgramme>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = channel_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT c.id, p.title, p.subtitle, p.start_ts, p.stop_ts, p.category
         FROM epg_programmes p JOIN channels c ON c.epg_channel_id = p.channel_id
         WHERE c.id IN ({placeholders}) AND p.start_ts < ? AND p.stop_ts > ?
         ORDER BY c.id, p.start_ts"
    );
    let mut args: Vec<Box<dyn ToSql>> = channel_ids.iter().map(|id| Box::new(*id) as Box<dyn ToSql>).collect();
    args.push(Box::new(to_ts));
    args.push(Box::new(from_ts));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        Ok(GridProgramme {
            channel_id: row.get(0)?,
            title: row.get(1)?,
            subtitle: row.get(2)?,
            start_ts: row.get(3)?,
            stop_ts: row.get(4)?,
            category: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

// ─────────────── Continuar viendo, recientes, favoritos ───────────────

/// `None` si el perfil no tiene PIN configurado todavía (control parental sin activar:
/// no hay nada que ocultar). Ver `util::parental`.
pub fn get_profile_pin_hash(conn: &Connection, profile_id: i64) -> Result<Option<String>> {
    conn.query_row("SELECT pin_hash FROM profiles WHERE id = ?1", params![profile_id], |row| row.get(0))
        .optional()
        .map(|opt| opt.flatten())
        .map_err(Into::into)
}

/// Usado solo por pruebas que necesitan un `profile_id` válido para sembrar datos y no
/// les importa cuál — desde que los perfiles son un concepto real (F "perfiles
/// Netflix"), ningún camino de producción llama a esto: crear un perfil en silencio
/// dejaría de tener sentido con un selector explícito delante.
#[cfg(test)]
pub fn ensure_default_profile(conn: &Connection) -> Result<i64> {
    if let Some(id) = conn.query_row("SELECT id FROM profiles ORDER BY id LIMIT 1", [], |row| row.get(0)).optional()? {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO profiles (name, adult_ok, created_at, updated_at) VALUES ('Perfil', 0, ?1, ?1)",
        params![now_epoch()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Vista de un perfil para la interfaz: nunca lleva `pin_hash` (aunque no es una
/// credencial de proveedor, sigue sin ser algo que la API deba devolver tal cual).
pub struct ProfileRow {
    pub id: i64,
    pub name: String,
    pub avatar: Option<String>,
    pub has_pin: bool,
    pub adult_ok: bool,
    pub created_at: i64,
}

fn row_to_profile(row: &Row) -> rusqlite::Result<ProfileRow> {
    let pin_hash: Option<String> = row.get("pin_hash")?;
    Ok(ProfileRow {
        id: row.get("id")?,
        name: row.get("name")?,
        avatar: row.get("avatar")?,
        has_pin: pin_hash.is_some(),
        adult_ok: row.get::<_, i64>("adult_ok")? != 0,
        created_at: row.get("created_at")?,
    })
}

pub fn list_profiles(conn: &Connection) -> Result<Vec<ProfileRow>> {
    let mut stmt = conn.prepare("SELECT * FROM profiles ORDER BY created_at ASC")?;
    let rows = stmt.query_map([], row_to_profile)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn get_profile(conn: &Connection, id: i64) -> Result<Option<ProfileRow>> {
    conn.query_row("SELECT * FROM profiles WHERE id = ?1", params![id], row_to_profile).optional().map_err(Into::into)
}

pub fn count_profiles(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM profiles", [], |row| row.get(0)).map_err(Into::into)
}

/// El UUID de Supabase de este perfil, si ya se sincronizó alguna vez. Hace falta
/// leerlo antes de borrar la fila local (`server::profiles::delete_profile_route`):
/// una vez borrada, ya no habría de dónde sacarlo para borrar también el espejo remoto.
pub fn get_profile_remote_id(conn: &Connection, profile_id: i64) -> Result<Option<String>> {
    conn.query_row("SELECT remote_id FROM profiles WHERE id = ?1", params![profile_id], |row| row.get(0))
        .optional()
        .map(|opt| opt.flatten())
        .map_err(Into::into)
}

/// Si el perfil tiene `adult_ok = 0` (infantil), nunca ve categorías adultas — sin PIN
/// de por medio, no hay nada que desbloquear (ver F "perfiles Netflix" en
/// docs/DECISIONS.md, sustituye al `ParentalGuard` global de F8).
pub fn profile_allows_adult(conn: &Connection, profile_id: i64) -> Result<bool> {
    conn.query_row("SELECT adult_ok FROM profiles WHERE id = ?1", params![profile_id], |row| row.get::<_, i64>(0))
        .optional()
        .map(|opt| opt.unwrap_or(0) != 0)
        .map_err(Into::into)
}

// ─────────────── Sincronización: item_key estable entre dispositivos ───────────────
//
// El `id` local (autoincremental de SQLite) depende del orden en que se sincronizó
// ESTE PC — no significa nada en otro dispositivo con otro catálogo. `item_key` usa
// `match_key` (F3, estable con independencia del proveedor) para que un favorito o un
// progreso tengan sentido al bajarlos en otro PC. Los canales de directo no tienen
// equivalente razonable (cada proveedor los numera distinto) y se quedan fuera: solo
// locales, nunca se suben ni se bajan (ver `sync::engine`).

/// `item_key` estable para una película o serie ya sincronizada en este PC, o `None` si
/// el id no existe (pudo borrarse).
pub fn item_key_for(conn: &Connection, item_kind: &str, item_id: i64) -> Result<Option<String>> {
    match item_kind {
        "movie" => conn.query_row("SELECT match_key FROM movies WHERE id = ?1", params![item_id], |r| r.get(0)).optional(),
        "series" => conn.query_row("SELECT match_key FROM series WHERE id = ?1", params![item_id], |r| r.get(0)).optional(),
        "episode" => conn
            .query_row(
                "SELECT s.match_key || ':' || e.season || ':' || e.number
                 FROM episodes e
                 JOIN series_sources ss ON ss.id = e.series_source_id
                 JOIN series s ON s.id = ss.series_id
                 WHERE e.id = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .optional(),
        _ => Ok(None), // "channel": sin item_key portable a propósito, ver comentario de la sección.
    }
    .map_err(Into::into)
}

/// El camino inverso: de un `item_key` bajado de Supabase al `id` local, si ese
/// contenido existe en el catálogo de este PC (puede que no — proveedores distintos).
pub fn item_id_for_key(conn: &Connection, item_kind: &str, item_key: &str) -> Result<Option<i64>> {
    match item_kind {
        "movie" => conn.query_row("SELECT id FROM movies WHERE match_key = ?1", params![item_key], |r| r.get(0)).optional(),
        "series" => conn.query_row("SELECT id FROM series WHERE match_key = ?1", params![item_key], |r| r.get(0)).optional(),
        "episode" => {
            let Some((series_key, season, number)) = split_episode_key(item_key) else { return Ok(None) };
            conn.query_row(
                "SELECT e.id FROM episodes e
                 JOIN series_sources ss ON ss.id = e.series_source_id
                 JOIN series s ON s.id = ss.series_id
                 WHERE s.match_key = ?1 AND e.season = ?2 AND e.number = ?3",
                params![series_key, season, number],
                |r| r.get(0),
            )
            .optional()
        }
        _ => Ok(None),
    }
    .map_err(Into::into)
}

/// `"titulo-normalizado-2020:4:5"` -> `("titulo-normalizado-2020", 4, 5)`. El
/// `match_key` de la serie puede llevar guiones pero nunca `:` (regla de
/// `catalog::normalize`), así que partir por los dos últimos `:` es seguro.
fn split_episode_key(item_key: &str) -> Option<(&str, i64, i64)> {
    let mut parts = item_key.rsplitn(3, ':');
    let number: i64 = parts.next()?.parse().ok()?;
    let season: i64 = parts.next()?.parse().ok()?;
    let series_key = parts.next()?;
    Some((series_key, season, number))
}

pub struct PlaybackItem {
    pub item_kind: String,
    pub item_id: i64,
    pub position_s: i64,
    pub duration_s: Option<i64>,
    pub finished: bool,
    pub played_at: i64,
}

pub fn upsert_playback_progress(
    conn: &Connection,
    profile_id: i64,
    item_kind: &str,
    item_id: i64,
    position_s: i64,
    duration_s: Option<i64>,
) -> Result<()> {
    // Terminado a partir del 90% (ver docs/PRD.md / F6); se calcula aquí porque es la
    // única escritura de progreso, no hace falta duplicarlo en cada llamador.
    let finished = duration_s.is_some_and(|d| d > 0 && (position_s as f64 / d as f64) >= 0.9);
    conn.execute(
        "INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, duration_s, finished, played_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(profile_id, item_kind, item_id) DO UPDATE SET
            position_s = excluded.position_s, duration_s = excluded.duration_s,
            finished = excluded.finished, played_at = excluded.played_at",
        params![profile_id, item_kind, item_id, position_s, duration_s, finished as i64, now_epoch()],
    )?;
    Ok(())
}

/// Progreso guardado de un ítem concreto: lo consulta el reproductor al abrir algo, para
/// ofrecer "reanudar desde X" (F6). `None` si nunca se guardó progreso para ese ítem.
pub fn get_playback_progress(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<Option<PlaybackItem>> {
    conn.query_row(
        "SELECT item_kind, item_id, position_s, duration_s, finished, played_at FROM playback_state
         WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
        params![profile_id, item_kind, item_id],
        |row| {
            Ok(PlaybackItem {
                item_kind: row.get(0)?,
                item_id: row.get(1)?,
                position_s: row.get(2)?,
                duration_s: row.get(3)?,
                finished: row.get::<_, i64>(4)? != 0,
                played_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn list_continue_watching_raw(conn: &Connection, profile_id: i64, limit: i64) -> Result<Vec<PlaybackItem>> {
    // Directo excluido a propósito: un canal no se "continúa", se sintoniza o no. Sin
    // este filtro, cada canal por el que se hace zapping aparecía aquí con 0% visto
    // para siempre (encontrado probando contra datos reales, F8).
    let mut stmt = conn.prepare(
        "SELECT item_kind, item_id, position_s, duration_s, finished, played_at FROM playback_state
         WHERE profile_id = ?1 AND finished = 0 AND item_kind != 'channel' ORDER BY played_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![profile_id, limit], |row| {
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

pub struct ContinueWatchingRow {
    pub item_kind: String,
    pub item_id: i64,
    pub position_s: i64,
    pub duration_s: Option<i64>,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster_path: Option<String>,
}

/// Fila de `continuar viendo` con título y portada de verdad (F8) — antes solo daba
/// `item_kind`/`item_id` en crudo y el frontend no tenía forma de pintar nada útil. Una
/// consulta por ítem (no una sola con `UNION`): son como mucho unas pocas decenas de
/// filas, y cada tipo (película/episodio/canal) se enriquece de una tabla distinta.
pub fn list_continue_watching(conn: &Connection, profile_id: i64, limit: i64) -> Result<Vec<ContinueWatchingRow>> {
    let raw = list_continue_watching_raw(conn, profile_id, limit)?;
    let mut items = Vec::with_capacity(raw.len());

    for r in raw {
        let enriched = match r.item_kind.as_str() {
            "movie" => conn
                .query_row("SELECT clean_title, poster_path FROM movies WHERE id = ?1", params![r.item_id], |row| {
                    Ok((row.get::<_, String>(0)?, None::<String>, row.get::<_, Option<String>>(1)?))
                })
                .optional()?,
            "episode" => conn
                .query_row(
                    "SELECT s.clean_title, e.season, e.number, e.title, e.still_path, s.poster_path
                     FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id JOIN series s ON s.id = ss.series_id
                     WHERE e.id = ?1",
                    params![r.item_id],
                    |row| {
                        let series_title: String = row.get(0)?;
                        let season: i64 = row.get(1)?;
                        let number: i64 = row.get(2)?;
                        let episode_title: Option<String> = row.get(3)?;
                        let still_path: Option<String> = row.get(4)?;
                        let series_poster: Option<String> = row.get(5)?;
                        let subtitle = format!("T{season} E{number}{}", episode_title.map(|t| format!(" · {t}")).unwrap_or_default());
                        Ok((series_title, Some(subtitle), still_path.or(series_poster)))
                    },
                )
                .optional()?,
            // "channel" no llega aquí: `list_continue_watching_raw` ya lo excluye
            // (directo no se "continúa"). El catch-all cubre igualmente cualquier otro
            // valor inesperado en `item_kind`.
            _ => None,
        };

        // El ítem pudo borrarse del catálogo desde que se guardó el progreso (una
        // resincronización que ya no lo trae): se salta en vez de enseñar un hueco.
        if let Some((title, subtitle, poster_path)) = enriched {
            items.push(ContinueWatchingRow { item_kind: r.item_kind, item_id: r.item_id, position_s: r.position_s, duration_s: r.duration_s, title, subtitle, poster_path });
        }
    }

    Ok(items)
}

pub struct NextEpisodeRow {
    pub series_id: i64,
    pub series_title: String,
    pub poster_path: Option<String>,
    pub episode_id: i64,
    pub season: i64,
    pub number: i64,
    pub episode_title: Option<String>,
    pub still_path: Option<String>,
}

/// Series en las que el perfil ha visto algo, con el episodio pendiente inmediatamente
/// después del último reproducido — "siguiente episodio pendiente por serie" de F8. No
/// es "reanudar donde lo dejaste" (eso ya es `continue_watching`): aquí el episodio en
/// sí puede no tener ningún progreso guardado todavía.
pub fn list_next_episodes(conn: &Connection, profile_id: i64, limit: i64) -> Result<Vec<NextEpisodeRow>> {
    let mut series_stmt = conn.prepare(
        "SELECT s.id, s.clean_title, s.poster_path, MAX(ps.played_at) AS last_played
         FROM playback_state ps
         JOIN episodes e ON e.id = ps.item_id AND ps.item_kind = 'episode'
         JOIN series_sources ss ON ss.id = e.series_source_id
         JOIN series s ON s.id = ss.series_id
         WHERE ps.profile_id = ?1
         GROUP BY s.id
         ORDER BY last_played DESC
         LIMIT ?2",
    )?;
    let series_rows: Vec<(i64, String, Option<String>)> = series_stmt
        .query_map(params![profile_id, limit], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut results = Vec::new();
    for (series_id, series_title, series_poster) in series_rows {
        let last_episode: Option<(i64, i64)> = conn
            .query_row(
                "SELECT e.season, e.number FROM playback_state ps
                 JOIN episodes e ON e.id = ps.item_id AND ps.item_kind = 'episode'
                 JOIN series_sources ss ON ss.id = e.series_source_id
                 WHERE ps.profile_id = ?1 AND ss.series_id = ?2
                 ORDER BY ps.played_at DESC LIMIT 1",
                params![profile_id, series_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((last_season, last_number)) = last_episode else { continue };

        let mut episodes_stmt = conn.prepare(
            "SELECT e.id, e.season, e.number, e.title, e.still_path
             FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id
             WHERE ss.series_id = ?1",
        )?;
        let all_episodes: Vec<(i64, i64, i64, Option<String>, Option<String>)> = episodes_stmt
            .query_map(params![series_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let Some((episode_id, season, number, episode_title, still_path)) = find_next_episode(&all_episodes, last_season, last_number) else {
            continue;
        };

        // El "siguiente" episodio es por definición uno que no se ha empezado; si ya
        // tiene su propio progreso guardado (se tocó fuera de orden, o es el mismo que
        // "continuar viendo" por cualquier motivo), no se sugiere aquí — se encontró
        // probando de verdad: sin este filtro, la fila podía reanudar a mitad en vez de
        // empezar desde el principio, confundiendo las dos filas entre sí.
        let already_has_progress: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM playback_state WHERE profile_id = ?1 AND item_kind = 'episode' AND item_id = ?2)",
            params![profile_id, episode_id],
            |row| row.get(0),
        )?;
        if already_has_progress {
            continue;
        }

        results.push(NextEpisodeRow { series_id, series_title, poster_path: series_poster, episode_id, season, number, episode_title, still_path });
    }

    Ok(results)
}

/// Misma lógica que `PlayerScreen.tsx`'s `findNextEpisode`: siguiente número dentro de
/// la misma temporada, o el primero de la siguiente temporada que tenga episodios.
fn find_next_episode(
    episodes: &[(i64, i64, i64, Option<String>, Option<String>)],
    last_season: i64,
    last_number: i64,
) -> Option<(i64, i64, i64, Option<String>, Option<String>)> {
    let mut same_season: Vec<_> = episodes.iter().filter(|(_, season, _, _, _)| *season == last_season).collect();
    same_season.sort_by_key(|(_, _, number, _, _)| *number);
    if let Some(next) = same_season.into_iter().find(|(_, _, number, _, _)| *number > last_number) {
        return Some(next.clone());
    }

    let mut seasons: Vec<i64> = episodes.iter().map(|(_, season, _, _, _)| *season).collect();
    seasons.sort_unstable();
    seasons.dedup();
    let next_season = seasons.into_iter().find(|s| *s > last_season)?;

    let mut next_season_episodes: Vec<_> = episodes.iter().filter(|(_, season, _, _, _)| *season == next_season).collect();
    next_season_episodes.sort_by_key(|(_, _, number, _, _)| *number);
    next_season_episodes.into_iter().next().cloned()
}

pub fn list_recent_movies(conn: &Connection, limit: i64) -> Result<Vec<MovieListItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, clean_title, year, poster_path, rating, tmdb_state FROM movies ORDER BY added_at DESC, id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(MovieListItem { id: row.get(0)?, clean_title: row.get(1)?, year: row.get(2)?, poster_path: row.get(3)?, rating: row.get(4)?, tmdb_state: row.get(5)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct FavoriteRow {
    pub item_kind: String,
    pub item_id: i64,
    pub title: String,
    pub poster_path: Option<String>,
}

/// Favoritos con título y portada de verdad (F8) — antes solo `item_kind`/`item_id` en
/// crudo, suficiente para el conmutador pero no para poder ver la lista en ningún sitio.
/// Solo canal/película/serie tienen sentido aquí (no episodios sueltos): coincide con
/// dónde vive el conmutador en la interfaz.
pub fn list_favorites(conn: &Connection, profile_id: i64) -> Result<Vec<FavoriteRow>> {
    let mut stmt = conn.prepare("SELECT item_kind, item_id FROM favorites WHERE profile_id = ?1 ORDER BY added_at DESC")?;
    let raw: Vec<(String, i64)> = stmt.query_map(params![profile_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;

    let mut items = Vec::with_capacity(raw.len());
    for (item_kind, item_id) in raw {
        let enriched: Option<(String, Option<String>)> = match item_kind.as_str() {
            "movie" => conn.query_row("SELECT clean_title, poster_path FROM movies WHERE id = ?1", params![item_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?,
            "series" => conn.query_row("SELECT clean_title, poster_path FROM series WHERE id = ?1", params![item_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?,
            "channel" => conn.query_row("SELECT display_name, NULL FROM channels WHERE id = ?1", params![item_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?,
            _ => None,
        };
        // El ítem pudo borrarse del catálogo desde que se marcó como favorito: se
        // salta, no se enseña un favorito roto.
        if let Some((title, poster_path)) = enriched {
            items.push(FavoriteRow { item_kind, item_id, title, poster_path });
        }
    }
    Ok(items)
}

pub fn add_favorite(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO favorites (profile_id, item_kind, item_id, added_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(profile_id, item_kind, item_id) DO NOTHING",
        params![profile_id, item_kind, item_id, now_epoch()],
    )?;
    Ok(())
}

pub fn remove_favorite(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM favorites WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
        params![profile_id, item_kind, item_id],
    )?;
    Ok(())
}

// ─────────────── Ajustes y modo LAN (F10) ───────────────

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM app_settings WHERE key = ?1", params![key], |row| row.get(0)).optional().map_err(Into::into)
}

pub struct PairedDeviceRow {
    pub id: i64,
    pub device_name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// Todos los dispositivos alguna vez emparejados, revocados incluidos (Ajustes los
/// enseña tachados en vez de hacerlos desaparecer — conservar el historial es a
/// propósito, ver `queries::revoke_paired_device`). Nunca incluye el token: no se
/// guarda en claro en ningún sitio después de entregarlo una vez.
pub fn list_paired_devices(conn: &Connection) -> Result<Vec<PairedDeviceRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, device_name, created_at, last_seen_at, revoked_at FROM paired_devices ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PairedDeviceRow {
                id: row.get(0)?,
                device_name: row.get(1)?,
                created_at: row.get(2)?,
                last_seen_at: row.get(3)?,
                revoked_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Resuelve un token de dispositivo LAN a su id, solo si no ha sido revocado. El
/// middleware de autenticación (`server::lan`) llama a esto en cada petición no local;
/// recibe ya el hash (`util::pairing::hash_token`), nunca el token en claro.
pub fn find_active_device_by_token_hash(conn: &Connection, token_hash: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM paired_devices WHERE token_hash = ?1 AND revoked_at IS NULL",
        params![token_hash],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{self, ProviderKind, TitleFields};
    use crate::db::schema::open_connection_at;
    use crate::sources::model::{RawMovie, RawSeries};
    use std::collections::HashMap;
    use std::path::Path;

    fn memory_conn() -> Connection {
        open_connection_at(Path::new(":memory:")).unwrap()
    }

    fn seed_movie(conn: &Connection, provider_id: i64, title: &str, year: Option<i32>, genres: Option<&str>, quality: Option<&str>) -> i64 {
        let raw = RawMovie {
            remote_id: title.to_string(),
            category_remote_id: None,
            raw_title: title.to_string(),
            container_extension: None,
            logo_url: None,
            added_at: Some(now_epoch()),
            rating: None,
            tmdb_id: None,
        };
        let match_key = format!("{}-{}", title.to_lowercase().replace(' ', "-"), year.map(|y| y.to_string()).unwrap_or_default());
        let fields = TitleFields { clean_title: title, year, match_key: &match_key, quality, language: None };
        queries::upsert_movie(conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
        let id: i64 = conn.query_row("SELECT id FROM movies WHERE match_key = ?1", params![match_key], |r| r.get(0)).unwrap();
        if let Some(g) = genres {
            conn.execute("UPDATE movies SET genres = ?1 WHERE id = ?2", params![g, id]).unwrap();
        }
        id
    }

    #[test]
    fn item_key_for_movie_roundtrips_through_match_key() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Mi Película", Some(2020), None, None);

        // `seed_movie` (helper de test) no normaliza acentos como catalog::normalize sí
        // hace de verdad — solo importa que item_key_for devuelva el match_key tal cual
        // se guardó, y que el camino inverso encuentre la misma fila.
        let key = item_key_for(&conn, "movie", movie_id).unwrap().unwrap();
        assert_eq!(key, "mi-película-2020");
        assert_eq!(item_id_for_key(&conn, "movie", &key).unwrap(), Some(movie_id));
    }

    #[test]
    fn item_key_for_unknown_id_is_none_not_an_error() {
        let conn = memory_conn();
        assert_eq!(item_key_for(&conn, "movie", 9999).unwrap(), None);
        assert_eq!(item_id_for_key(&conn, "movie", "no-existe-2020").unwrap(), None);
    }

    #[test]
    fn item_key_for_channel_is_always_none() {
        // Sin item_key portable a propósito: cada proveedor numera/nombra distinto.
        let conn = memory_conn();
        assert_eq!(item_key_for(&conn, "channel", 1).unwrap(), None);
    }

    #[test]
    fn item_key_for_episode_roundtrips_including_series_with_hyphens() {
        use crate::sources::model::RawEpisode;

        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series = RawSeries { remote_id: "s1".into(), raw_title: "La Casa de Papel".into(), cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "La Casa de Papel", year: None, match_key: "la-casa-de-papel-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &series, &fields).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources WHERE remote_id = 's1'", [], |r| r.get(0)).unwrap();

        let episode = RawEpisode { remote_id: "e1".into(), season: 4, number: 5, title: None, container_extension: None, duration_s: None, still_url: None };
        upsert_episodes(&conn, series_source_id, &[episode]).unwrap();
        let episode_id: i64 = conn.query_row("SELECT id FROM episodes WHERE remote_id = 'e1'", [], |r| r.get(0)).unwrap();

        let key = item_key_for(&conn, "episode", episode_id).unwrap().unwrap();
        assert_eq!(key, "la-casa-de-papel-0000:4:5");
        assert_eq!(item_id_for_key(&conn, "episode", &key).unwrap(), Some(episode_id));
    }

    #[test]
    fn split_episode_key_handles_hyphenated_series_match_keys() {
        assert_eq!(split_episode_key("la-casa-de-papel-2020:4:5"), Some(("la-casa-de-papel-2020", 4, 5)));
        assert_eq!(split_episode_key("no-tiene-suficientes-partes"), None);
        assert_eq!(split_episode_key(""), None);
    }

    #[test]
    fn list_movies_hides_adult_category_sources_unless_unlocked() {
        use crate::sources::model::MediaKind;
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let adult_cat = queries::upsert_categories(
            &conn,
            provider_id,
            MediaKind::Movie,
            &[crate::sources::model::RawCategory { remote_id: "18".into(), kind: MediaKind::Movie, name: "XXX".into() }],
        )
        .unwrap();
        conn.execute("UPDATE categories SET is_adult = 1 WHERE id = ?1", params![adult_cat["18"]]).unwrap();

        seed_movie(&conn, provider_id, "Peli Normal", Some(2020), None, None);
        let adult_id = seed_movie(&conn, provider_id, "Peli Adulta", Some(2020), None, None);
        conn.execute("UPDATE movie_sources SET category_id = ?1 WHERE movie_id = ?2", params![adult_cat["18"], adult_id]).unwrap();

        let hidden = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, 50, false).unwrap();
        assert!(hidden.iter().all(|m| m.clean_title != "Peli Adulta"));
        assert!(hidden.iter().any(|m| m.clean_title == "Peli Normal"));

        let unlocked = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, 50, true).unwrap();
        assert!(unlocked.iter().any(|m| m.clean_title == "Peli Adulta"));
    }

    #[test]
    fn list_movies_filters_by_genre_via_json_each() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Accion Uno", Some(2020), Some("[28,12]"), None);
        seed_movie(&conn, provider_id, "Drama Uno", Some(2020), Some("[18]"), None);

        let action = list_movies_page(&conn, Some(28), None, None, MovieSort::AddedDesc, None, 50, true).unwrap();
        assert_eq!(action.len(), 1);
        assert_eq!(action[0].clean_title, "Accion Uno");
    }

    #[test]
    fn list_movies_filters_by_year_and_quality() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Peli 2020 4K", Some(2020), None, Some("4K"));
        seed_movie(&conn, provider_id, "Peli 2021 HD", Some(2021), None, Some("HD"));

        let by_year = list_movies_page(&conn, None, Some(2020), None, MovieSort::AddedDesc, None, 50, true).unwrap();
        assert_eq!(by_year.len(), 1);

        let by_quality = list_movies_page(&conn, None, None, Some("4K"), MovieSort::AddedDesc, None, 50, true).unwrap();
        assert_eq!(by_quality.len(), 1);
        assert_eq!(by_quality[0].clean_title, "Peli 2020 4K");
    }

    #[test]
    fn list_movies_pagination_cursor_excludes_seen_ids() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        for i in 0..5 {
            seed_movie(&conn, provider_id, &format!("Peli {i}"), Some(2020), None, None);
        }

        let first_page = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, 2, true).unwrap();
        assert_eq!(first_page.len(), 2);
        let cursor = first_page.last().unwrap().id;

        let second_page = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, Some(cursor), 2, true).unwrap();
        assert_eq!(second_page.len(), 2);
        assert!(second_page.iter().all(|m| m.id != cursor));
    }

    #[test]
    fn get_movie_includes_sources() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let id = seed_movie(&conn, provider_id, "Peli", Some(2020), Some("[28]"), None);

        let detail = get_movie(&conn, id).unwrap().unwrap();
        assert_eq!(detail.clean_title, "Peli");
        assert_eq!(detail.genres, vec![28]);
        assert_eq!(detail.sources.len(), 1);
    }

    #[test]
    fn get_movie_returns_none_when_missing() {
        let conn = memory_conn();
        assert!(get_movie(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn search_finds_movies_channels_and_series_after_rebuild() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Aventura Espacial", Some(2020), None, None);

        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Aventura Serie".into(), cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Aventura Serie", year: None, match_key: "aventura-serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &series_raw, &fields).unwrap();

        queries::rebuild_search_index(&conn).unwrap();

        let hits = search(&conn, "Aventura", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.item_kind == "movie"));
        assert!(hits.iter().any(|h| h.item_kind == "series"));
    }

    #[test]
    fn search_with_garbage_input_returns_empty_not_error() {
        let conn = memory_conn();
        queries::rebuild_search_index(&conn).unwrap();
        let hits = search(&conn, "***", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn playback_progress_marks_finished_at_90_percent() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        upsert_playback_progress(&conn, profile_id, "movie", 1, 85, Some(100)).unwrap();

        let finished: i64 = conn.query_row("SELECT finished FROM playback_state WHERE profile_id=?1", params![profile_id], |r| r.get(0)).unwrap();
        assert_eq!(finished, 0);

        upsert_playback_progress(&conn, profile_id, "movie", 1, 95, Some(100)).unwrap();
        let finished: i64 = conn.query_row("SELECT finished FROM playback_state WHERE profile_id=?1", params![profile_id], |r| r.get(0)).unwrap();
        assert_eq!(finished, 1);
    }

    #[test]
    fn get_playback_progress_returns_none_when_never_played() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        assert!(get_playback_progress(&conn, profile_id, "movie", 1).unwrap().is_none());

        upsert_playback_progress(&conn, profile_id, "movie", 1, 42, Some(100)).unwrap();
        let progress = get_playback_progress(&conn, profile_id, "movie", 1).unwrap().unwrap();
        assert_eq!(progress.position_s, 42);
        assert!(!progress.finished);
    }

    #[test]
    fn continue_watching_excludes_finished_items() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie1 = seed_movie(&conn, provider_id, "Sin terminar", Some(2020), None, None);
        let movie2 = seed_movie(&conn, provider_id, "Terminada", Some(2020), None, None);
        upsert_playback_progress(&conn, profile_id, "movie", movie1, 50, Some(100)).unwrap(); // no terminado
        upsert_playback_progress(&conn, profile_id, "movie", movie2, 99, Some(100)).unwrap(); // terminado

        let continuing = list_continue_watching(&conn, profile_id, 10).unwrap();
        assert_eq!(continuing.len(), 1);
        assert_eq!(continuing[0].item_id, movie1);
        assert_eq!(continuing[0].title, "Sin terminar");
    }

    #[test]
    fn continue_watching_skips_items_whose_catalog_entry_was_deleted() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        // item_id 999 no existe en `movies`: se saltó de una resincronización.
        upsert_playback_progress(&conn, profile_id, "movie", 999, 10, Some(100)).unwrap();
        assert!(list_continue_watching(&conn, profile_id, 10).unwrap().is_empty());
    }

    #[test]
    fn continue_watching_excludes_channels() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        // Zapping real: cada canal visto queda con position_s=0 para siempre. No debe
        // aparecer en "continuar viendo" (un canal no se "continúa").
        upsert_playback_progress(&conn, profile_id, "channel", 1, 0, None).unwrap();
        assert!(list_continue_watching(&conn, profile_id, 10).unwrap().is_empty());
    }

    #[test]
    fn continue_watching_enriches_episodes_with_series_title_and_still() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Mi Serie".into(), cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Mi Serie", year: None, match_key: "mi-serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &series_raw, &fields).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources", [], |r| r.get(0)).unwrap();

        let episode = crate::sources::model::RawEpisode {
            remote_id: "e1".into(),
            season: 2,
            number: 5,
            title: Some("El regreso".into()),
            container_extension: Some("mp4".into()),
            duration_s: Some(1200),
            still_url: Some("/still.jpg".into()),
        };
        upsert_episodes(&conn, series_source_id, &[episode]).unwrap();
        let episode_id: i64 = conn.query_row("SELECT id FROM episodes", [], |r| r.get(0)).unwrap();

        upsert_playback_progress(&conn, profile_id, "episode", episode_id, 30, Some(1200)).unwrap();

        let continuing = list_continue_watching(&conn, profile_id, 10).unwrap();
        assert_eq!(continuing.len(), 1);
        assert_eq!(continuing[0].title, "Mi Serie");
        assert_eq!(continuing[0].subtitle.as_deref(), Some("T2 E5 · El regreso"));
        assert_eq!(continuing[0].poster_path.as_deref(), Some("/still.jpg"));
    }

    fn seed_series_with_episodes(conn: &Connection, provider_id: i64, title: &str, episodes: &[(i64, i64)]) -> (i64, Vec<i64>) {
        let series_raw = RawSeries { remote_id: title.into(), raw_title: title.into(), cover_url: None, plot: None, release_date: None };
        let match_key = format!("{}-0000", title.to_lowercase().replace(' ', "-"));
        let fields = TitleFields { clean_title: title, year: None, match_key: &match_key, quality: None, language: None };
        queries::upsert_series(conn, provider_id, &series_raw, &fields).unwrap();
        let series_id: i64 = conn.query_row("SELECT id FROM series WHERE match_key = ?1", params![match_key], |r| r.get(0)).unwrap();
        let series_source_id: i64 =
            conn.query_row("SELECT id FROM series_sources WHERE series_id = ?1", params![series_id], |r| r.get(0)).unwrap();

        let raw_episodes: Vec<_> = episodes
            .iter()
            .map(|(season, number)| crate::sources::model::RawEpisode {
                remote_id: format!("s{season}e{number}"),
                season: *season as u32,
                number: *number as u32,
                title: None,
                container_extension: Some("mp4".into()),
                duration_s: Some(1200),
                still_url: None,
            })
            .collect();
        upsert_episodes(conn, series_source_id, &raw_episodes).unwrap();

        let mut stmt = conn.prepare("SELECT id, season, number FROM episodes WHERE series_source_id = ?1").unwrap();
        let mut rows: Vec<(i64, i64, i64)> =
            stmt.query_map(params![series_source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        rows.sort_by_key(|(_, season, number)| (*season, *number));
        (series_id, rows.into_iter().map(|(id, _, _)| id).collect())
    }

    #[test]
    fn list_next_episodes_finds_the_one_right_after_the_last_played() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (series_id, episode_ids) = seed_series_with_episodes(&conn, provider_id, "Mi Serie", &[(1, 1), (1, 2), (1, 3)]);

        upsert_playback_progress(&conn, profile_id, "episode", episode_ids[0], 1200, Some(1200)).unwrap();

        let next = list_next_episodes(&conn, profile_id, 10).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].series_id, series_id);
        assert_eq!(next[0].episode_id, episode_ids[1]);
        assert_eq!((next[0].season, next[0].number), (1, 2));
    }

    #[test]
    fn list_next_episodes_crosses_into_the_next_season() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (_, episode_ids) = seed_series_with_episodes(&conn, provider_id, "Mi Serie", &[(1, 1), (1, 2), (2, 1)]);

        upsert_playback_progress(&conn, profile_id, "episode", episode_ids[1], 1200, Some(1200)).unwrap();

        let next = list_next_episodes(&conn, profile_id, 10).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].episode_id, episode_ids[2]);
        assert_eq!((next[0].season, next[0].number), (2, 1));
    }

    #[test]
    fn list_next_episodes_omits_series_already_at_the_last_episode() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (_, episode_ids) = seed_series_with_episodes(&conn, provider_id, "Mi Serie", &[(1, 1), (1, 2)]);

        upsert_playback_progress(&conn, profile_id, "episode", episode_ids[1], 1200, Some(1200)).unwrap();

        assert!(list_next_episodes(&conn, profile_id, 10).unwrap().is_empty());
    }

    #[test]
    fn list_next_episodes_skips_a_candidate_that_already_has_its_own_progress() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (_, episode_ids) = seed_series_with_episodes(&conn, provider_id, "Mi Serie", &[(1, 1), (1, 2), (1, 3)]);

        upsert_playback_progress(&conn, profile_id, "episode", episode_ids[0], 1200, Some(1200)).unwrap();
        // El "siguiente" (episode_ids[1]) ya se tocó fuera de orden en algún momento:
        // no debe sugerirse como "empieza desde cero" — confundiría con "continuar viendo".
        upsert_playback_progress(&conn, profile_id, "episode", episode_ids[1], 300, Some(1200)).unwrap();

        assert!(list_next_episodes(&conn, profile_id, 10).unwrap().is_empty());
    }

    #[test]
    fn favorites_add_is_idempotent_and_remove_works() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Favorita", Some(2020), None, None);

        add_favorite(&conn, profile_id, "movie", movie_id).unwrap();
        add_favorite(&conn, profile_id, "movie", movie_id).unwrap(); // repetir: no debe fallar

        let favorites = list_favorites(&conn, profile_id).unwrap();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].title, "Favorita");

        remove_favorite(&conn, profile_id, "movie", movie_id).unwrap();
        assert!(list_favorites(&conn, profile_id).unwrap().is_empty());
    }

    #[test]
    fn favorites_skips_entries_whose_catalog_item_was_deleted() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        // item_id 999 no existe en `movies`.
        add_favorite(&conn, profile_id, "movie", 999).unwrap();
        assert!(list_favorites(&conn, profile_id).unwrap().is_empty());
    }

    #[test]
    fn ensure_default_profile_is_idempotent() {
        let conn = memory_conn();
        let first = ensure_default_profile(&conn).unwrap();
        let second = ensure_default_profile(&conn).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn list_categories_filters_by_kind() {
        use crate::sources::model::{MediaKind, RawCategory};
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        queries::upsert_categories(&conn, provider_id, MediaKind::Live, &[RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() }]).unwrap();
        queries::upsert_categories(&conn, provider_id, MediaKind::Movie, &[RawCategory { remote_id: "2".into(), kind: MediaKind::Movie, name: "ACCION".into() }]).unwrap();

        let live = list_categories(&conn, "live", true).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "ESPAÑA");
    }

    #[test]
    fn list_categories_hides_adult_ones_unless_unlocked() {
        use crate::sources::model::{MediaKind, RawCategory};
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let map = queries::upsert_categories(
            &conn,
            provider_id,
            MediaKind::Live,
            &[
                RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() },
                RawCategory { remote_id: "2".into(), kind: MediaKind::Live, name: "XXX".into() },
            ],
        )
        .unwrap();
        conn.execute("UPDATE categories SET is_adult = 1 WHERE id = ?1", params![map["2"]]).unwrap();

        let hidden = list_categories(&conn, "live", false).unwrap();
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].name, "ESPAÑA");

        let unlocked = list_categories(&conn, "live", true).unwrap();
        assert_eq!(unlocked.len(), 2);
    }

    #[test]
    fn list_channels_hides_adult_category_channels_unless_unlocked() {
        use crate::sources::model::{MediaKind, RawCategory, RawChannel};
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let map = queries::upsert_categories(
            &conn,
            provider_id,
            MediaKind::Live,
            &[RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "XXX".into() }],
        )
        .unwrap();
        conn.execute("UPDATE categories SET is_adult = 1 WHERE id = ?1", params![map["1"]]).unwrap();

        let channel = RawChannel {
            remote_id: "1".into(),
            category_remote_id: Some("1".into()),
            raw_name: "Canal Adulto".into(),
            logo_url: None,
            epg_channel_id: None,
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal Adulto", year: None, match_key: "n/a", quality: None, language: None };
        queries::upsert_channel(&conn, provider_id, &map, &channel, &fields).unwrap();

        assert!(list_channels_page(&conn, None, None, None, 50, false).unwrap().is_empty());
        assert_eq!(list_channels_page(&conn, None, None, None, 50, true).unwrap().len(), 1);
    }

    #[test]
    fn profile_pin_hash_roundtrip() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        assert_eq!(get_profile_pin_hash(&conn, profile_id).unwrap(), None);

        queries::set_profile_pin_hash(&conn, profile_id, Some("hash-de-prueba")).unwrap();
        assert_eq!(get_profile_pin_hash(&conn, profile_id).unwrap().as_deref(), Some("hash-de-prueba"));

        queries::set_profile_pin_hash(&conn, profile_id, None).unwrap();
        assert_eq!(get_profile_pin_hash(&conn, profile_id).unwrap(), None);
    }

    #[test]
    fn resolve_channel_stream_returns_provider_and_remote_id() {
        use crate::sources::model::RawChannel;
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let channel = RawChannel {
            remote_id: "101".into(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: None,
            epg_channel_id: None,
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();

        let channel_id: i64 = conn.query_row("SELECT id FROM channels WHERE remote_id = '101'", [], |r| r.get(0)).unwrap();
        let info = resolve_channel_stream(&conn, channel_id).unwrap().unwrap();
        assert_eq!(info.provider_id, provider_id);
        assert_eq!(info.remote_id, "101");

        assert!(resolve_channel_stream(&conn, 9999).unwrap().is_none());
    }

    #[test]
    fn resolve_movie_stream_picks_specific_or_first_source() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Peli", Some(2020), None, Some("HD"));

        let raw_4k = RawMovie {
            remote_id: "extra-4k".into(),
            category_remote_id: None,
            raw_title: "Peli 4K".into(),
            container_extension: Some("mkv".into()),
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: None,
        };
        let fields_4k = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-2020", quality: Some("4K"), language: None };
        queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw_4k, &fields_4k).unwrap();

        let first = resolve_movie_stream(&conn, movie_id, None).unwrap().unwrap();
        assert_eq!(first.provider_id, provider_id);

        let source_4k_id: i64 =
            conn.query_row("SELECT id FROM movie_sources WHERE remote_id = 'extra-4k'", [], |r| r.get(0)).unwrap();
        let specific = resolve_movie_stream(&conn, movie_id, Some(source_4k_id)).unwrap().unwrap();
        assert_eq!(specific.remote_id, "extra-4k");
        assert_eq!(specific.container_extension.as_deref(), Some("mkv"));

        assert!(resolve_movie_stream(&conn, 9999, None).unwrap().is_none());
    }

    #[test]
    fn resolve_episode_stream_joins_series_sources_for_provider_id() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series_raw = RawSeries { remote_id: "9001".into(), raw_title: "Serie".into(), cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Serie", year: None, match_key: "serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &series_raw, &fields).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources", [], |r| r.get(0)).unwrap();

        let episode = crate::sources::model::RawEpisode {
            remote_id: "9001-1-1".into(),
            season: 1,
            number: 1,
            title: Some("Piloto".into()),
            container_extension: Some("mp4".into()),
            duration_s: Some(1200),
            still_url: None,
        };
        upsert_episodes(&conn, series_source_id, &[episode]).unwrap();
        let episode_id: i64 = conn.query_row("SELECT id FROM episodes", [], |r| r.get(0)).unwrap();

        let info = resolve_episode_stream(&conn, episode_id).unwrap().unwrap();
        assert_eq!(info.provider_id, provider_id);
        assert_eq!(info.remote_id, "9001-1-1");
        assert_eq!(info.container_extension.as_deref(), Some("mp4"));

        assert!(resolve_episode_stream(&conn, 9999).unwrap().is_none());
    }

    #[test]
    fn list_channels_redacts_credentials_embedded_in_logo_url() {
        use crate::sources::model::RawChannel;
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let channel = RawChannel {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: Some("http://panel.example.com/live/juan/hunter2/logo.png".into()),
            epg_channel_id: None,
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();

        let channels = list_channels_page(&conn, None, None, None, 50, true).unwrap();
        let logo = channels[0].logo_url.as_deref().unwrap();
        assert!(!logo.contains("juan"));
        assert!(!logo.contains("hunter2"));
    }

    fn seed_channel_with_epg_id(conn: &Connection, provider_id: i64, remote_id: &str, epg_channel_id: Option<&str>) -> i64 {
        use crate::sources::model::RawChannel;
        let channel = RawChannel {
            remote_id: remote_id.into(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: None,
            epg_channel_id: epg_channel_id.map(String::from),
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        queries::upsert_channel(conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();
        conn.query_row("SELECT id FROM channels WHERE provider_id = ?1 AND remote_id = ?2", params![provider_id, remote_id], |r| r.get(0)).unwrap()
    }

    fn insert_test_programme(conn: &Connection, channel_id: &str, start_ts: i64, stop_ts: i64, title: &str) {
        conn.execute(
            "INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title) VALUES (?1, ?2, ?3, ?4)",
            params![channel_id, start_ts, stop_ts, title],
        )
        .unwrap();
    }

    #[test]
    fn epg_now_and_next_finds_current_and_upcoming_programme() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let channel_id = seed_channel_with_epg_id(&conn, provider_id, "1", Some("canaluno.es"));

        insert_test_programme(&conn, "canaluno.es", 1000, 2000, "Ahora");
        insert_test_programme(&conn, "canaluno.es", 2000, 3000, "Después");

        let results = epg_now_and_next(&conn, &[channel_id], 1500).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].current.as_ref().unwrap().title, "Ahora");
        assert_eq!(results[0].next.as_ref().unwrap().title, "Después");
    }

    #[test]
    fn epg_now_and_next_returns_none_without_epg_channel_id_or_guide() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let without_id = seed_channel_with_epg_id(&conn, provider_id, "1", None);
        let without_guide = seed_channel_with_epg_id(&conn, provider_id, "2", Some("singuia.es"));

        let results = epg_now_and_next(&conn, &[without_id, without_guide], 1500).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.current.is_none() && r.next.is_none()));
    }

    #[test]
    fn epg_grid_returns_only_overlapping_programmes_for_requested_channels() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let channel_a = seed_channel_with_epg_id(&conn, provider_id, "1", Some("a.es"));
        let _channel_b = seed_channel_with_epg_id(&conn, provider_id, "2", Some("b.es"));

        insert_test_programme(&conn, "a.es", 1000, 2000, "Dentro de la ventana");
        insert_test_programme(&conn, "a.es", 5000, 6000, "Fuera de la ventana");
        insert_test_programme(&conn, "b.es", 1000, 2000, "De otro canal, no pedido");

        let grid = epg_grid(&conn, &[channel_a], 500, 2500).unwrap();
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].title, "Dentro de la ventana");
        assert_eq!(grid[0].channel_id, channel_a);
    }

    #[test]
    fn epg_grid_returns_empty_for_no_channels() {
        let conn = memory_conn();
        assert!(epg_grid(&conn, &[], 0, 1000).unwrap().is_empty());
    }
}
