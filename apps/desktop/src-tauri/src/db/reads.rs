//! Consultas de lectura para la API HTTP de F4. Separadas de `queries.rs` (que cubre la
//! escritura del sincronizador) por claridad; siguen siendo el único sitio con SQL para
//! estos datos.
//!
//! Paginación por cursor, nunca offset (regla R4). Para el orden natural (recientes,
//! `id DESC`) el cursor es solo el último `id` visto. Para cualquier otro orden
//! (título, año, nota — F "premium": filtros reales en Películas/Series) el `id` solo
//! no basta como cursor: hacía falta también el valor de la propia columna de orden,
//! comparado como valor compuesto `(columna, id) < (?, ?)` — si no, "después del id
//! 500" no dice nada sobre dónde seguir cuando se ordena por título. Esto era una
//! limitación conocida y documentada desde F4 ("sin una interfaz real que lo ejercite
//! todavía"); con los filtros de verdad ya en la interfaz, tocaba arreglarlo de verdad
//! en vez de seguir aproximando con solo el `id`.

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, ToSql};
use std::collections::HashMap;

use crate::util::redact;

use super::{now_epoch, Result};

pub struct CategoryRow {
    pub id: i64,
    pub name: String,
}

/// `include_adult` en `false` oculta las categorías marcadas `is_adult` (control
/// parental, F8): mientras no se desbloquee con el PIN, ni siquiera aparecen en el
/// selector de categorías.
pub struct GenreNameRow {
    pub id: i64,
    pub name: String,
}

/// Nombres de género de TMDB para el selector de filtros de Películas/Series — sin
/// esto, lo único que hay son los ids numéricos que `movies.genres`/`series.genres`
/// guardan tal cual vienen de TMDB. Vacío hasta la primera sincronización con clave de
/// TMDB (`commands::spawn_tmdb_matching`), no es un error.
pub struct SeedMovieRow {
    pub id: i64,
    pub clean_title: String,
    pub tmdb_id: i64,
}

/// Películas "semilla" para "Recomendado para ti" (F "premium"): favoritas, o vistas
/// pasado el 70% sin abandonar — las más recientes primero. Solo las que ya tienen
/// `tmdb_id` (sin eso no hay nada que preguntarle a TMDB). Si una está en ambos sitios
/// a la vez, cuenta una sola vez, con la fecha más reciente de las dos.
pub fn list_recommendation_seed_movies(conn: &Connection, profile_id: i64, limit: i64) -> Result<Vec<SeedMovieRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.clean_title, m.tmdb_id FROM (
            SELECT item_id AS movie_id, added_at AS seed_time FROM favorites WHERE profile_id = ?1 AND item_kind = 'movie'
            UNION ALL
            SELECT item_id AS movie_id, played_at AS seed_time FROM playback_state
                WHERE profile_id = ?1 AND item_kind = 'movie' AND duration_s IS NOT NULL AND duration_s > 0
                    AND (CAST(position_s AS REAL) / duration_s) > 0.7
         ) seeds
         JOIN movies m ON m.id = seeds.movie_id
         WHERE m.tmdb_id IS NOT NULL
         GROUP BY m.id
         ORDER BY MAX(seeds.seed_time) DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![profile_id, limit], |row| {
        Ok(SeedMovieRow { id: row.get(0)?, clean_title: row.get(1)?, tmdb_id: row.get(2)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct RecommendedMovieRow {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    /// Título de la película semilla que motivó esta sugerencia ("Porque te gustó X").
    pub because_of: String,
}

/// Cruza los ids de TMDB que sugirió `TmdbClient::movie_recommendations` con el
/// catálogo local — la regla de oro del informe de I+D: recomendar algo que el
/// proveedor no tiene sería el error clásico. `candidates` ya viene en orden de
/// prioridad (semillas más recientes primero); se descartan las semillas mismas (ya
/// vistas/favoritas, no tiene sentido "recomendarlas") y los duplicados.
pub fn resolve_recommendations(
    conn: &Connection,
    candidates: &[(i64, String)],
    exclude_movie_ids: &[i64],
    limit: i64,
) -> Result<Vec<RecommendedMovieRow>> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for (tmdb_id, because_of) in candidates {
        if results.len() as i64 >= limit {
            break;
        }
        let row: Option<(i64, String, Option<i32>, Option<String>, Option<f64>)> = conn
            .query_row(
                "SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path, rating FROM movies WHERE tmdb_id = ?1",
                params![tmdb_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((id, clean_title, year, poster_path, rating)) = row {
            if exclude_movie_ids.contains(&id) || !seen.insert(id) {
                continue;
            }
            results.push(RecommendedMovieRow { id, clean_title, year, poster_path, rating, because_of: because_of.clone() });
        }
    }
    Ok(results)
}

pub struct SeedSeriesRow {
    pub id: i64,
    pub clean_title: String,
    pub tmdb_id: i64,
}

/// Igual que `list_recommendation_seed_movies`, para series: favoritas, o con al menos
/// un episodio terminado (`playback_state.finished`, ya calculado por
/// `upsert_playback_progress`/`set_episode_watched` al 90% o al marcarlo a mano — no hay
/// una única "posición/duración" de la serie entera como sí la hay en una película).
pub fn list_recommendation_seed_series(conn: &Connection, profile_id: i64, limit: i64) -> Result<Vec<SeedSeriesRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.clean_title, s.tmdb_id FROM (
            SELECT item_id AS series_id, added_at AS seed_time FROM favorites WHERE profile_id = ?1 AND item_kind = 'series'
            UNION ALL
            SELECT ss.series_id AS series_id, MAX(ps.played_at) AS seed_time
            FROM playback_state ps
            JOIN episodes e ON e.id = ps.item_id
            JOIN series_sources ss ON ss.id = e.series_source_id
            WHERE ps.profile_id = ?1 AND ps.item_kind = 'episode' AND ps.finished = 1
            GROUP BY ss.series_id
         ) seeds
         JOIN series s ON s.id = seeds.series_id
         WHERE s.tmdb_id IS NOT NULL
         GROUP BY s.id
         ORDER BY MAX(seeds.seed_time) DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![profile_id, limit], |row| {
        Ok(SeedSeriesRow { id: row.get(0)?, clean_title: row.get(1)?, tmdb_id: row.get(2)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct RecommendedSeriesRow {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
}

/// Igual que `resolve_recommendations`, para series.
pub fn resolve_series_recommendations(
    conn: &Connection,
    candidates: &[(i64, String)],
    exclude_series_ids: &[i64],
    limit: i64,
) -> Result<Vec<RecommendedSeriesRow>> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for (tmdb_id, because_of) in candidates {
        if results.len() as i64 >= limit {
            break;
        }
        let row: Option<(i64, String, Option<i32>, Option<String>, Option<f64>)> = conn
            .query_row(
                "SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path, rating FROM series WHERE tmdb_id = ?1",
                params![tmdb_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((id, clean_title, year, poster_path, rating)) = row {
            if exclude_series_ids.contains(&id) || !seen.insert(id) {
                continue;
            }
            results.push(RecommendedSeriesRow { id, clean_title, year, poster_path, rating, because_of: because_of.clone() });
        }
    }
    Ok(results)
}

pub struct SimilarTitleRow {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
}

/// "Títulos similares" en la ficha de una película (F "premium"): mismo cruce contra el
/// catálogo local que `resolve_recommendations` (nunca sugerir algo que el proveedor no
/// tiene), pero a partir de un solo título en vez de varias semillas ponderadas, así que
/// no hace falta el "porque te gustó X" ni las exclusiones múltiples.
pub fn resolve_similar_movies(conn: &Connection, tmdb_ids: &[i64], exclude_movie_id: i64, limit: i64) -> Result<Vec<SimilarTitleRow>> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for tmdb_id in tmdb_ids {
        if results.len() as i64 >= limit {
            break;
        }
        let row: Option<(i64, String, Option<i32>, Option<String>)> = conn
            .query_row("SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path FROM movies WHERE tmdb_id = ?1", params![tmdb_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        if let Some((id, clean_title, year, poster_path)) = row {
            if id == exclude_movie_id || !seen.insert(id) {
                continue;
            }
            results.push(SimilarTitleRow { id, clean_title, year, poster_path });
        }
    }
    Ok(results)
}

/// Igual que `resolve_similar_movies`, para la ficha de serie.
pub fn resolve_similar_series(conn: &Connection, tmdb_ids: &[i64], exclude_series_id: i64, limit: i64) -> Result<Vec<SimilarTitleRow>> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for tmdb_id in tmdb_ids {
        if results.len() as i64 >= limit {
            break;
        }
        let row: Option<(i64, String, Option<i32>, Option<String>)> = conn
            .query_row("SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path FROM series WHERE tmdb_id = ?1", params![tmdb_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        if let Some((id, clean_title, year, poster_path)) = row {
            if id == exclude_series_id || !seen.insert(id) {
                continue;
            }
            results.push(SimilarTitleRow { id, clean_title, year, poster_path });
        }
    }
    Ok(results)
}

pub fn list_genre_names(conn: &Connection, kind: &str) -> Result<Vec<GenreNameRow>> {
    let mut stmt = conn.prepare("SELECT tmdb_genre_id, name FROM genre_names WHERE kind = ?1 ORDER BY name")?;
    let rows = stmt.query_map(params![kind], |row| Ok(GenreNameRow { id: row.get(0)?, name: row.get(1)? }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

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

pub struct CategoryAdultRow {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub is_adult: bool,
    pub manual: bool,
}

/// Todas las categorías de todos los tipos, para marcar a mano las de adultos (Ajustes).
pub fn list_categories_for_adult_settings(conn: &Connection) -> Result<Vec<CategoryAdultRow>> {
    let mut stmt = conn.prepare("SELECT id, kind, name, is_adult, is_adult_manual IS NOT NULL FROM categories ORDER BY kind, name")?;
    let rows = stmt.query_map([], |row| {
        Ok(CategoryAdultRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            is_adult: row.get::<_, i64>(3)? != 0,
            manual: row.get(4)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct ChannelListItem {
    pub id: i64,
    pub display_name: String,
    pub logo_url: Option<String>,
    pub category_id: Option<i64>,
    pub has_catchup: bool,
    /// Número del canal en el proveedor (`sort_order`, el `num` de Xtream).
    pub number: Option<i64>,
}

/// Posición tras la que seguir en `list_channels_page`: el número del último canal visto
/// (`None` si no tenía) y su id. Tiene que seguir el mismo orden que la consulta; antes
/// se paginaba por id ordenando por número, y una página podía saltarse canales o
/// repetirlos en cuanto los ids no seguían el orden de los números.
pub type ChannelCursor = (Option<i64>, i64);

#[allow(clippy::too_many_arguments)]
pub fn list_channels_page(
    conn: &Connection,
    category_id: Option<i64>,
    search: Option<&str>,
    after: Option<ChannelCursor>,
    limit: i64,
    include_adult: bool,
) -> Result<Vec<ChannelListItem>> {
    let mut sql = format!("SELECT {CHANNEL_LIST_COLUMNS} FROM channels c WHERE 1=1");
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if !include_adult {
        sql.push_str(NOT_ADULT_CHANNEL);
    }
    if let Some(cat) = category_id {
        sql.push_str(" AND c.category_id = ?");
        args.push(Box::new(cat));
    }
    if let Some(q) = search {
        sql.push_str(" AND c.display_name LIKE ? ESCAPE '\\'");
        args.push(Box::new(like_pattern(q)));
    }
    // Orden: primero los que tienen número, por número y luego id; después los que no.
    match after {
        Some((Some(number), id)) => {
            sql.push_str(" AND (c.sort_order IS NULL OR c.sort_order > ? OR (c.sort_order = ? AND c.id > ?))");
            args.push(Box::new(number));
            args.push(Box::new(number));
            args.push(Box::new(id));
        }
        Some((None, id)) => {
            sql.push_str(" AND c.sort_order IS NULL AND c.id > ?");
            args.push(Box::new(id));
        }
        None => {}
    }
    sql.push_str(" ORDER BY c.sort_order IS NULL, c.sort_order, c.id LIMIT ?");
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), channel_list_item)?;
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
    pub added_at: Option<i64>,
}

/// Cursor de una página: siempre lleva el último `id` visto, y además el valor de la
/// propia columna de orden cuando el orden no es "recientes" — sin eso, "sigue después
/// del id 500" no basta para saber por dónde continuar ordenando por título/año/nota.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageCursor {
    pub id: i64,
    pub key: Option<CursorKey>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorKey {
    Text,
    Int(i32),
    Float(f64),
}

#[allow(clippy::too_many_arguments)]
pub fn list_movies_page(
    conn: &Connection,
    genre: Option<i64>,
    year: Option<i32>,
    quality: Option<&str>,
    sort: MovieSort,
    after: Option<PageCursor>,
    after_text: Option<&str>,
    limit: i64,
    access: ProfileAccess,
) -> Result<Vec<MovieListItem>> {
    let mut sql = String::from("SELECT DISTINCT m.id, m.clean_title, m.year, COALESCE(m.poster_path, m.provider_poster_url) AS poster_path, m.rating, m.tmdb_state, m.added_at FROM movies m");
    if quality.is_some() {
        sql.push_str(" JOIN movie_sources ms ON ms.movie_id = m.id");
    }
    sql.push_str(" WHERE 1=1");

    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    sql.push_str(&title_access_sql(TitleTable::Movies, "m", access));
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

    let order = match sort {
        MovieSort::AddedDesc => "m.id DESC",
        MovieSort::TitleAsc => "m.clean_title ASC, m.id ASC",
        MovieSort::YearDesc => "m.year DESC, m.id DESC",
        MovieSort::RatingDesc => "m.rating DESC, m.id DESC",
    };
    if let Some(cursor) = after {
        match (sort, cursor.key) {
            (MovieSort::AddedDesc, _) => {
                sql.push_str(" AND m.id < ?");
                args.push(Box::new(cursor.id));
            }
            (MovieSort::TitleAsc, Some(CursorKey::Text)) => {
                sql.push_str(" AND (m.clean_title, m.id) > (?, ?)");
                args.push(Box::new(after_text.unwrap_or_default().to_string()));
                args.push(Box::new(cursor.id));
            }
            (MovieSort::YearDesc, Some(CursorKey::Int(y))) => {
                sql.push_str(" AND (m.year, m.id) < (?, ?)");
                args.push(Box::new(y));
                args.push(Box::new(cursor.id));
            }
            (MovieSort::RatingDesc, Some(CursorKey::Float(r))) => {
                sql.push_str(" AND (m.rating, m.id) < (?, ?)");
                args.push(Box::new(r));
                args.push(Box::new(cursor.id));
            }
            // La última película vista no tenía año/nota (NULL): SQLite ordena los NULL
            // al final en DESC, así que seguir significa quedarse dentro de ese mismo
            // grupo sin dato, por id — sin este caso, un cursor con clave `None` no
            // filtraría nada y la página se repetiría en bucle.
            (MovieSort::YearDesc, None) => {
                sql.push_str(" AND m.year IS NULL AND m.id < ?");
                args.push(Box::new(cursor.id));
            }
            (MovieSort::RatingDesc, None) => {
                sql.push_str(" AND m.rating IS NULL AND m.id < ?");
                args.push(Box::new(cursor.id));
            }
            // Cursor sin la clave que le toca a este orden (petición inconsistente, o
            // primera página): se ignora en vez de fallar la consulta entera.
            _ => {}
        }
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
            added_at: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Construye el cursor de la página siguiente a partir del último ítem devuelto — el
/// mismo tipo de dato en las dos puntas (aquí y donde se compara) evita que se cuele un
/// desajuste entre cómo se genera el cursor y cómo se interpreta.
pub fn movie_page_cursor(sort: MovieSort, last: &MovieListItem) -> (PageCursor, Option<String>) {
    match sort {
        MovieSort::AddedDesc => (PageCursor { id: last.id, key: None }, None),
        MovieSort::TitleAsc => (PageCursor { id: last.id, key: Some(CursorKey::Text) }, Some(last.clean_title.clone())),
        // El texto (segundo elemento) es lo único que de verdad viaja al cliente como
        // `next_cursor_key` — sin él, la próxima petición no tendría forma de reconstruir
        // el valor de año/nota, solo el id, y el filtro del cursor no se aplicaría nunca.
        MovieSort::YearDesc => (PageCursor { id: last.id, key: last.year.map(CursorKey::Int) }, last.year.map(|y| y.to_string())),
        MovieSort::RatingDesc => (PageCursor { id: last.id, key: last.rating.map(CursorKey::Float) }, last.rating.map(|r| r.to_string())),
    }
}

pub struct MovieSourceItem {
    pub id: i64,
    pub provider_id: i64,
    pub remote_id: String,
    pub quality: Option<String>,
    pub language: Option<String>,
    pub container: Option<String>,
    pub duration_s: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub vod_info_checked_at: Option<i64>,
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
    /// `None` si `tmdb_state != "matched"`: sin `tmdb_id` no hay nada que preguntarle a
    /// TMDB sobre el tráiler.
    pub tmdb_id: Option<i64>,
    pub youtube_trailer_key: Option<String>,
    pub trailer_checked_at: Option<i64>,
    pub age_rating: Option<String>,
    pub similar_tmdb_ids: Vec<i64>,
    pub sources: Vec<MovieSourceItem>,
}

/// Deshace `queries::join_tmdb_ids` (migración 009: texto separado por comas, no una
/// tabla propia). Entradas que no sean un número entero se descartan en vez de romper
/// la ficha entera — no debería pasar nunca, pero un dato así de accesorio no merece un
/// error si algún día lo hace.
fn parse_tmdb_ids(raw: Option<String>) -> Vec<i64> {
    raw.map(|s| s.split(',').filter_map(|part| part.trim().parse().ok()).collect()).unwrap_or_default()
}

pub fn get_movie(conn: &Connection, id: i64) -> Result<Option<MovieDetail>> {
    let base = conn
        .query_row(
            "SELECT id, clean_title, year, overview, COALESCE(poster_path, provider_poster_url) AS poster_path, backdrop_path, rating, runtime_min, genres, tmdb_state,
                    tmdb_id, youtube_trailer_key, trailer_checked_at, age_rating, similar_tmdb_ids
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
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                ))
            },
        )
        .optional()?;

    let Some((
        id,
        clean_title,
        year,
        overview,
        poster_path,
        backdrop_path,
        rating,
        runtime_min,
        genres_json,
        tmdb_state,
        tmdb_id,
        youtube_trailer_key,
        trailer_checked_at,
        age_rating,
        similar_tmdb_ids_raw,
    )) = base
    else {
        return Ok(None);
    };

    let genres: Vec<i64> = genres_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let similar_tmdb_ids = parse_tmdb_ids(similar_tmdb_ids_raw);

    let mut stmt = conn.prepare(
        "SELECT id, provider_id, remote_id, quality, language, container, duration_s, video_codec, audio_codec, vod_info_checked_at
         FROM movie_sources WHERE movie_id = ?1",
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
                duration_s: row.get(6)?,
                video_codec: row.get(7)?,
                audio_codec: row.get(8)?,
                vod_info_checked_at: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(MovieDetail {
        id,
        clean_title,
        year,
        overview,
        poster_path,
        backdrop_path,
        rating,
        runtime_min,
        genres,
        tmdb_state,
        tmdb_id,
        youtube_trailer_key,
        trailer_checked_at,
        age_rating,
        similar_tmdb_ids,
        sources,
    }))
}

pub struct SeriesListItem {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub tmdb_state: String,
}

/// Igual que `MovieSort`: se comparten los cuatro órdenes, pero es un tipo aparte porque
/// series no tiene calidad como filtro (`series_sources` no la guarda por episodio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesSort {
    AddedDesc,
    TitleAsc,
    YearDesc,
    RatingDesc,
}

#[allow(clippy::too_many_arguments)]
pub fn list_series_page(
    conn: &Connection,
    genre: Option<i64>,
    year: Option<i32>,
    sort: SeriesSort,
    after: Option<PageCursor>,
    after_text: Option<&str>,
    limit: i64,
    access: ProfileAccess,
) -> Result<Vec<SeriesListItem>> {
    let mut sql = String::from("SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path, rating, tmdb_state FROM series WHERE 1=1");
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    sql.push_str(&title_access_sql(TitleTable::Series, "series", access));
    if let Some(g) = genre {
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(genres) WHERE value = ?)");
        args.push(Box::new(g));
    }
    if let Some(y) = year {
        sql.push_str(" AND year = ?");
        args.push(Box::new(y));
    }

    let order = match sort {
        SeriesSort::AddedDesc => "id DESC",
        SeriesSort::TitleAsc => "clean_title ASC, id ASC",
        SeriesSort::YearDesc => "year DESC, id DESC",
        SeriesSort::RatingDesc => "rating DESC, id DESC",
    };
    if let Some(cursor) = after {
        match (sort, cursor.key) {
            (SeriesSort::AddedDesc, _) => {
                sql.push_str(" AND id < ?");
                args.push(Box::new(cursor.id));
            }
            (SeriesSort::TitleAsc, Some(CursorKey::Text)) => {
                sql.push_str(" AND (clean_title, id) > (?, ?)");
                args.push(Box::new(after_text.unwrap_or_default().to_string()));
                args.push(Box::new(cursor.id));
            }
            (SeriesSort::YearDesc, Some(CursorKey::Int(y))) => {
                sql.push_str(" AND (year, id) < (?, ?)");
                args.push(Box::new(y));
                args.push(Box::new(cursor.id));
            }
            (SeriesSort::RatingDesc, Some(CursorKey::Float(r))) => {
                sql.push_str(" AND (rating, id) < (?, ?)");
                args.push(Box::new(r));
                args.push(Box::new(cursor.id));
            }
            (SeriesSort::YearDesc, None) => {
                sql.push_str(" AND year IS NULL AND id < ?");
                args.push(Box::new(cursor.id));
            }
            (SeriesSort::RatingDesc, None) => {
                sql.push_str(" AND rating IS NULL AND id < ?");
                args.push(Box::new(cursor.id));
            }
            _ => {}
        }
    }
    sql.push_str(&format!(" ORDER BY {order} LIMIT ?"));
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        Ok(SeriesListItem { id: row.get(0)?, clean_title: row.get(1)?, year: row.get(2)?, poster_path: row.get(3)?, rating: row.get(4)?, tmdb_state: row.get(5)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn series_page_cursor(sort: SeriesSort, last: &SeriesListItem) -> (PageCursor, Option<String>) {
    match sort {
        SeriesSort::AddedDesc => (PageCursor { id: last.id, key: None }, None),
        SeriesSort::TitleAsc => (PageCursor { id: last.id, key: Some(CursorKey::Text) }, Some(last.clean_title.clone())),
        SeriesSort::YearDesc => (PageCursor { id: last.id, key: last.year.map(CursorKey::Int) }, last.year.map(|y| y.to_string())),
        SeriesSort::RatingDesc => (PageCursor { id: last.id, key: last.rating.map(CursorKey::Float) }, last.rating.map(|r| r.to_string())),
    }
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
    pub tmdb_id: Option<i64>,
    pub youtube_trailer_key: Option<String>,
    pub trailer_checked_at: Option<i64>,
    pub age_rating: Option<String>,
    pub similar_tmdb_ids: Vec<i64>,
    pub sources: Vec<SeriesSourceItem>,
    pub episodes: Vec<EpisodeItem>,
}

pub fn get_series(conn: &Connection, id: i64) -> Result<Option<SeriesDetail>> {
    let base = conn
        .query_row(
            "SELECT id, clean_title, year, overview, COALESCE(poster_path, provider_poster_url) AS poster_path, backdrop_path, rating, genres, tmdb_state,
                    tmdb_id, youtube_trailer_key, trailer_checked_at, age_rating, similar_tmdb_ids
             FROM series WHERE id = ?1",
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
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                ))
            },
        )
        .optional()?;

    let Some((
        id,
        clean_title,
        year,
        overview,
        poster_path,
        backdrop_path,
        rating,
        genres_json,
        tmdb_state,
        tmdb_id,
        youtube_trailer_key,
        trailer_checked_at,
        age_rating,
        similar_tmdb_ids_raw,
    )) = base
    else {
        return Ok(None);
    };
    let genres: Vec<i64> = genres_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let similar_tmdb_ids = parse_tmdb_ids(similar_tmdb_ids_raw);

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

    Ok(Some(SeriesDetail {
        id,
        clean_title,
        year,
        overview,
        poster_path,
        backdrop_path,
        rating,
        genres,
        tmdb_state,
        tmdb_id,
        youtube_trailer_key,
        trailer_checked_at,
        age_rating,
        similar_tmdb_ids,
        sources,
        episodes,
    }))
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
        // La miniatura es una URL del proveedor: redactada antes de guardarla (R5).
        let still_url = ep.still_url.as_deref().map(redact);
        stmt.execute(params![series_source_id, ep.remote_id, ep.season, ep.number, ep.title, still_url, ep.duration_s, ep.container_extension])?;
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
    pub poster_path: Option<String>,
}

/// `search_index` (FTS5) solo guarda título — sin portada, la búsqueda era una lista de
/// texto plano (F "premium"). Se busca aparte, tabla por tabla, en vez de meter la
/// portada en el propio índice: así no hay que reconstruir el FTS5 entero cada vez que
/// cambia una portada por un reemparejamiento de TMDB.
pub fn search(conn: &Connection, query: &str, limit_per_kind: i64, access: ProfileAccess) -> Result<Vec<SearchHit>> {
    let mut stmt = conn.prepare(
        "SELECT item_kind, item_id, title FROM search_index WHERE search_index MATCH ?1
         ORDER BY rank LIMIT ?2",
    )?;
    // FTS5 necesita una consulta bien formada; una entrada vacía o con solo espacios no
    // debe reventar, solo no encontrar nada.
    let fts_query = format!("{}*", fts_escape(query));
    let rows = stmt
        .query_map(params![fts_query, limit_per_kind * 3], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>());

    let raw_hits: Vec<(String, i64, String)> = match rows {
        Ok(hits) => hits,
        Err(_) => return Ok(Vec::new()), // consulta FTS5 mal formada (p. ej. solo símbolos): sin resultados, no error 500
    };

    // Lo que el perfil no puede ver no sale, ni por búsqueda (F14). Se pidieron de más
    // (`limit_per_kind * 3`) para que filtrar no deje la lista corta.
    let mut visible = Vec::with_capacity(raw_hits.len());
    for hit in raw_hits {
        if item_allowed(conn, &hit.0, hit.1, access)? {
            visible.push(hit);
        }
    }
    Ok(visible
        .into_iter()
        .map(|(item_kind, item_id, title)| {
            let poster_path = match item_kind.as_str() {
                "movie" => query_optional_string(conn, "SELECT COALESCE(poster_path, provider_poster_url) AS poster_path FROM movies WHERE id = ?1", item_id),
                "series" => query_optional_string(conn, "SELECT COALESCE(poster_path, provider_poster_url) AS poster_path FROM series WHERE id = ?1", item_id),
                // El logo de canal es una URL del proveedor: se redacta antes de salir de
                // la capa de datos, igual que en `list_channels_page` (regla R5).
                "channel" => query_optional_string(conn, "SELECT logo_url FROM channels WHERE id = ?1", item_id).map(|url| redact(&url)),
                _ => None,
            };
            SearchHit { item_kind, item_id, title, poster_path }
        })
        .collect())
}

/// `SELECT <columna nullable> FROM t WHERE id = ?1` → `Option<String>`, aplanando el
/// "no hay fila" (`query_row` + `.optional()`) y el "hay fila pero la columna es NULL"
/// (la propia columna) en un único `None` — el llamador no necesita distinguirlos.
fn query_optional_string(conn: &Connection, sql: &str, id: i64) -> Option<String> {
    conn.query_row(sql, params![id], |r| r.get::<_, Option<String>>(0)).optional().ok().flatten().flatten()
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
        let channel: Option<(Option<String>, i64)> = conn
            .query_row("SELECT epg_channel_id, provider_id FROM channels WHERE id = ?1", params![channel_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;

        let Some((Some(epg_channel_id), provider_id)) = channel else {
            results.push(NowNextProgramme { channel_id, current: None, next: None });
            continue;
        };

        let current = conn
            .query_row(
                "SELECT title, subtitle, start_ts, stop_ts FROM epg_programmes
                 WHERE channel_id = ?1 AND (provider_id IS NULL OR provider_id = ?3) AND start_ts <= ?2 AND stop_ts > ?2
                 ORDER BY start_ts LIMIT 1",
                params![epg_channel_id, now, provider_id],
                programme_row,
            )
            .optional()?;
        let next = conn
            .query_row(
                "SELECT title, subtitle, start_ts, stop_ts FROM epg_programmes
                 WHERE channel_id = ?1 AND (provider_id IS NULL OR provider_id = ?3) AND start_ts > ?2 ORDER BY start_ts LIMIT 1",
                params![epg_channel_id, now, provider_id],
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
         FROM epg_programmes p JOIN channels c ON c.epg_channel_id = p.channel_id AND (p.provider_id IS NULL OR p.provider_id = c.provider_id)
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
/// PIN (hasheados) de los perfiles sin límite de edad: los que pueden autorizar que otro
/// perfil vea más (F14). No hace falta que vean categorías XXX: un padre sin ellas
/// sigue siendo quien decide.
pub fn adult_pin_hashes(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT pin_hash FROM profiles WHERE max_age IS NULL AND pin_hash IS NOT NULL")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

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
    pub max_age: Option<i64>,
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
        max_age: row.get("max_age")?,
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

/// El id en la nube de este perfil, si ya se encoló alguna vez. Hace falta
/// leerlo antes de borrar la fila local (`server::profiles::delete_profile_route`):
/// una vez borrada, ya no habría de dónde sacarlo para borrar también el espejo remoto.
pub fn get_profile_remote_id(conn: &Connection, profile_id: i64) -> Result<Option<String>> {
    conn.query_row("SELECT remote_id FROM profiles WHERE id = ?1", params![profile_id], |row| row.get(0))
        .optional()
        .map(|opt| opt.flatten())
        .map_err(Into::into)
}

// ─────────────── Qué puede ver cada perfil (F8, F14) ───────────────

/// Restricciones de un perfil: contenido de categorías adultas y edad máxima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileAccess {
    pub include_adult: bool,
    /// `None` es sin límite. Con límite, solo títulos con edad mínima conocida y apta.
    pub max_age: Option<i64>,
}

impl ProfileAccess {
    pub const UNRESTRICTED: Self = Self { include_adult: true, max_age: None };
    pub const NO_ADULT: Self = Self { include_adult: false, max_age: None };
}

/// Restricciones del perfil. Un perfil que no existe ve lo mínimo, nunca todo.
pub fn profile_access(conn: &Connection, profile_id: i64) -> Result<ProfileAccess> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row("SELECT adult_ok, max_age FROM profiles WHERE id = ?1", params![profile_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((adult_ok, max_age)) => ProfileAccess { include_adult: adult_ok != 0, max_age },
        None => ProfileAccess { include_adult: false, max_age: Some(0) },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleTable {
    Movies,
    Series,
}

/// Condiciones SQL (con `AND` delante) para que un título de `table`, con alias `alias`,
/// sea visible con `access`. Los valores son enteros, nunca texto del usuario.
pub fn title_access_sql(table: TitleTable, alias: &str, access: ProfileAccess) -> String {
    let mut sql = String::new();
    if !access.include_adult {
        // Se oculta si tiene alguna fuente en una categoría adulta: mejor ocultar de más
        // que enseñar de menos en un filtro parental (F8).
        let (sources, fk) = match table {
            TitleTable::Movies => ("movie_sources", "movie_id"),
            TitleTable::Series => ("series_sources", "series_id"),
        };
        sql.push_str(&format!(
            " AND NOT EXISTS (SELECT 1 FROM {sources} src_adult JOIN categories cat_adult ON cat_adult.id = src_adult.category_id
                              WHERE src_adult.{fk} = {alias}.id AND cat_adult.is_adult = 1)"
        ));
    }
    if let Some(max_age) = access.max_age {
        sql.push_str(&format!(" AND {alias}.age_min IS NOT NULL AND {alias}.age_min <= {max_age}"));
    }
    sql
}

/// Si el perfil puede ver ese elemento (`movie`, `series`, `episode` o `channel`). Para
/// lo que no está en una lista paginada: fichas, reproducción, filas de Inicio...
pub fn item_allowed(conn: &Connection, item_kind: &str, item_id: i64, access: ProfileAccess) -> Result<bool> {
    if access == ProfileAccess::UNRESTRICTED {
        return Ok(true);
    }
    let sql = match item_kind {
        "movie" => format!("SELECT EXISTS(SELECT 1 FROM movies m WHERE m.id = ?1{})", title_access_sql(TitleTable::Movies, "m", access)),
        "series" => format!("SELECT EXISTS(SELECT 1 FROM series s WHERE s.id = ?1{})", title_access_sql(TitleTable::Series, "s", access)),
        "episode" => format!(
            "SELECT EXISTS(SELECT 1 FROM episodes e JOIN series_sources ss ON ss.id = e.series_source_id JOIN series s ON s.id = ss.series_id
                           WHERE e.id = ?1{})",
            title_access_sql(TitleTable::Series, "s", access)
        ),
        "channel" if !access.include_adult => {
            "SELECT EXISTS(SELECT 1 FROM channels c WHERE c.id = ?1
                           AND (c.category_id IS NULL OR c.category_id NOT IN (SELECT id FROM categories WHERE is_adult = 1)))"
                .to_string()
        }
        "channel" => return Ok(true),
        _ => return Ok(false),
    };
    conn.query_row(&sql, params![item_id], |row| row.get(0)).map_err(Into::into)
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

/// El camino inverso: de un `item_key` bajado de la nube al `id` local, si ese
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
                .query_row("SELECT clean_title, COALESCE(poster_path, provider_poster_url) AS poster_path FROM movies WHERE id = ?1", params![r.item_id], |row| {
                    Ok((row.get::<_, String>(0)?, None::<String>, row.get::<_, Option<String>>(1)?))
                })
                .optional()?,
            "episode" => conn
                .query_row(
                    "SELECT s.clean_title, e.season, e.number, e.title, e.still_path, COALESCE(s.poster_path, s.provider_poster_url) AS poster_path
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
        "SELECT s.id, s.clean_title, COALESCE(s.poster_path, s.provider_poster_url) AS poster_path, MAX(ps.played_at) AS last_played
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

pub struct FeaturedMovieRow {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
}

/// Película destacada para el banner de Inicio: entre las añadidas más recientemente,
/// la mejor valorada que además tenga fondo y sinopsis de TMDB — sin eso el banner se
/// vería vacío o sin texto que enseñar, así que mejor no destacarla en absoluto. Se
/// mira entre las 50 más recientes (no todo el catálogo) para que sea algo con lo que
/// el usuario probablemente se vaya a encontrar, no una rareza de hace años.
pub fn get_featured_movie(conn: &Connection) -> Result<Option<FeaturedMovieRow>> {
    conn.query_row(
        "SELECT id, clean_title, year, overview, COALESCE(poster_path, provider_poster_url) AS poster_path, backdrop_path, rating
         FROM (SELECT * FROM movies WHERE tmdb_state = 'matched' ORDER BY added_at DESC, id DESC LIMIT 50)
         WHERE backdrop_path IS NOT NULL AND overview IS NOT NULL
         ORDER BY rating DESC NULLS LAST
         LIMIT 1",
        [],
        |row| {
            Ok(FeaturedMovieRow {
                id: row.get(0)?,
                clean_title: row.get(1)?,
                year: row.get(2)?,
                overview: row.get(3)?,
                poster_path: row.get(4)?,
                backdrop_path: row.get(5)?,
                rating: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn list_recent_movies(conn: &Connection, limit: i64) -> Result<Vec<MovieListItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path, rating, tmdb_state, added_at FROM movies ORDER BY added_at DESC, id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(MovieListItem {
            id: row.get(0)?,
            clean_title: row.get(1)?,
            year: row.get(2)?,
            poster_path: row.get(3)?,
            rating: row.get(4)?,
            tmdb_state: row.get(5)?,
            added_at: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct GenreRow {
    pub id: i64,
    pub movies: Vec<MovieListItem>,
}

/// Filas por género en Inicio (F "premium"): los géneros con más películas emparejadas
/// en el catálogo de verdad del usuario, no una lista fija — un proveedor centrado en
/// anime no debería ver una fila de "Documental" solo porque a alguien le pareció un
/// género típico. `top_genre_count` decide cuántas filas se intentan; una que resulte
/// vacía (rarísimo, pero el género pudo perder sus películas entre una sincronización y
/// otra) simplemente no aparece.
pub fn list_genre_rows(conn: &Connection, top_genre_count: i64, movies_per_genre: i64) -> Result<Vec<GenreRow>> {
    let mut genre_stmt = conn.prepare(
        "SELECT value AS genre_id, COUNT(*) AS n FROM movies, json_each(movies.genres)
         WHERE tmdb_state = 'matched' GROUP BY value ORDER BY n DESC LIMIT ?1",
    )?;
    let genre_ids: Vec<i64> = genre_stmt.query_map(params![top_genre_count], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;

    let mut movie_stmt = conn.prepare(
        "SELECT id, clean_title, year, COALESCE(poster_path, provider_poster_url) AS poster_path, rating, tmdb_state, added_at FROM movies
         WHERE tmdb_state = 'matched' AND EXISTS (SELECT 1 FROM json_each(movies.genres) WHERE value = ?1)
         ORDER BY added_at DESC, id DESC LIMIT ?2",
    )?;

    let mut rows = Vec::with_capacity(genre_ids.len());
    for genre_id in genre_ids {
        let movies = movie_stmt
            .query_map(params![genre_id, movies_per_genre], |row| {
                Ok(MovieListItem {
                    id: row.get(0)?,
                    clean_title: row.get(1)?,
                    year: row.get(2)?,
                    poster_path: row.get(3)?,
                    rating: row.get(4)?,
                    tmdb_state: row.get(5)?,
                    added_at: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !movies.is_empty() {
            rows.push(GenreRow { id: genre_id, movies });
        }
    }
    Ok(rows)
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
            "movie" => conn.query_row("SELECT clean_title, COALESCE(poster_path, provider_poster_url) AS poster_path FROM movies WHERE id = ?1", params![item_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?,
            "series" => conn.query_row("SELECT clean_title, COALESCE(poster_path, provider_poster_url) AS poster_path FROM series WHERE id = ?1", params![item_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?,
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

/// Ids de episodio ya vistos del todo (F "premium" — checkmark en la ficha de serie),
/// para un perfil y una serie concretos. `finished` ya lo calcula
/// `upsert_playback_progress` (90% o más), no hay que repetir el umbral aquí.
/// Progreso de un episodio para un perfil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeProgress {
    pub finished: bool,
    pub position_s: i64,
    pub played_at: i64,
}

/// Progreso de cada episodio de una serie (de cualquiera de sus fuentes) para un perfil,
/// por id de episodio: da el checkmark de "visto" y el "Continuar T·E" de la ficha.
pub fn list_episode_progress(conn: &Connection, profile_id: i64, series_id: i64) -> Result<HashMap<i64, EpisodeProgress>> {
    let mut stmt = conn.prepare(
        "SELECT ps.item_id, ps.finished, ps.position_s, ps.played_at FROM playback_state ps
         JOIN episodes e ON e.id = ps.item_id
         JOIN series_sources ss ON ss.id = e.series_source_id
         WHERE ps.profile_id = ?1 AND ps.item_kind = 'episode' AND ss.series_id = ?2",
    )?;
    let rows = stmt.query_map(params![profile_id, series_id], |row| {
        Ok((row.get::<_, i64>(0)?, EpisodeProgress { finished: row.get::<_, i64>(1)? != 0, position_s: row.get(2)?, played_at: row.get(3)? }))
    })?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>().map_err(Into::into)
}

/// El episodio que ofrece la ficha de serie ("Ver/Continuar T·E", como Netflix), con
/// `ordered` = ids en orden de temporada y número. Manda lo último que se tocó: empezado
/// sin terminar, ese mismo; terminado, el siguiente; tras el último, o sin nada visto, el
/// primero. El `bool` dice si el perfil ya iba por la serie ("Continuar" y no "Ver").
pub fn up_next_episode(ordered: &[i64], progress: &HashMap<i64, EpisodeProgress>) -> Option<(i64, bool)> {
    let first = *ordered.first()?;
    // A igual momento (marcar una temporada entera), cuenta el último en orden.
    let latest = ordered
        .iter()
        .enumerate()
        .filter_map(|(index, id)| progress.get(id).filter(|p| p.finished || p.position_s > 0).map(|p| (p.played_at, index, p.finished)))
        .max_by_key(|(played_at, index, _)| (*played_at, *index));
    match latest {
        None => Some((first, false)),
        Some((_, index, false)) => Some((ordered[index], true)),
        Some((_, index, true)) => match ordered.get(index + 1) {
            Some(next) => Some((*next, true)),
            None => Some((first, false)),
        },
    }
}

/// Quitar algo de "Continuar viendo" a mano (F "premium" — Netflix deja hacer esto,
/// aquí no había ninguna forma). Borra el progreso entero, no solo lo oculta: si se
/// vuelve a abrir el mismo título, empieza desde el principio otra vez, que es lo que
/// se espera al pedir "quitar de continuar viendo", no un simple filtro visual.
pub fn remove_playback_progress(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM playback_state WHERE profile_id = ?1 AND item_kind = ?2 AND item_id = ?3",
        params![profile_id, item_kind, item_id],
    )?;
    Ok(())
}

/// Ids de episodio de una temporada de una serie, en orden — usado por "marcar
/// temporada como vista" (F "premium": Plex/Emby dejan marcar sin reproducir, esta
/// app solo tenía el checkmark de lectura, ver `list_episode_progress`).
pub fn episode_ids_for_season(conn: &Connection, series_id: i64, season: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT e.id FROM episodes e
         JOIN series_sources ss ON ss.id = e.series_source_id
         WHERE ss.series_id = ?1 AND e.season = ?2
         ORDER BY e.number",
    )?;
    let rows = stmt.query_map(params![series_id, season], |row| row.get::<_, i64>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Marcar (o desmarcar) un episodio como visto a mano, sin haberlo reproducido de
/// verdad. Usa el mismo mecanismo que el progreso real (`playback_state.finished`),
/// para que "Continuar viendo" y el checkmark de la ficha no tengan que distinguir el
/// origen del dato. Al desmarcar, borra la fila entera — igual que
/// `remove_playback_progress`: no tiene sentido guardar "visto: no" de algo que nunca
/// se ha reproducido y podría no tener ni duración conocida.
pub fn set_episode_watched(conn: &Connection, profile_id: i64, episode_id: i64, watched: bool) -> Result<()> {
    if !watched {
        return remove_playback_progress(conn, profile_id, "episode", episode_id);
    }
    let duration_s: Option<i64> = conn
        .query_row("SELECT duration_s FROM episodes WHERE id = ?1", params![episode_id], |row| row.get(0))
        .optional()?
        .flatten();
    // Sin duración conocida (proveedor no la da), 1/1 sigue dando finished=true en
    // upsert_playback_progress (ratio 1.0) sin fingir una duración real que "Continuar
    // viendo" pudiera mostrar como progreso a medias.
    let duration_s = duration_s.filter(|d| *d > 0).unwrap_or(1);
    upsert_playback_progress(conn, profile_id, "episode", episode_id, duration_s, Some(duration_s))
}

/// Igual que `set_episode_watched`, para una película entera (F "premium" — paridad con
/// el marcado de episodios). La duración de TMDB (`movies.runtime_min`) es el dato más
/// fiable cuando existe; si no, se usa la de la primera fuente con `vod_info` ya
/// comprobado (F "premium" anterior); si ninguna de las dos existe todavía, 1/1 como en
/// episodios.
pub fn set_movie_watched(conn: &Connection, profile_id: i64, movie_id: i64, watched: bool) -> Result<()> {
    if !watched {
        return remove_playback_progress(conn, profile_id, "movie", movie_id);
    }
    let runtime_min: Option<i32> =
        conn.query_row("SELECT runtime_min FROM movies WHERE id = ?1", params![movie_id], |row| row.get(0)).optional()?.flatten();
    let source_duration: Option<i64> = conn
        .query_row(
            "SELECT duration_s FROM movie_sources WHERE movie_id = ?1 AND duration_s IS NOT NULL LIMIT 1",
            params![movie_id],
            |row| row.get(0),
        )
        .optional()?;
    let duration_s = runtime_min.filter(|m| *m > 0).map(|m| m as i64 * 60).or(source_duration).filter(|d| *d > 0).unwrap_or(1);
    upsert_playback_progress(conn, profile_id, "movie", movie_id, duration_s, Some(duration_s))
}

/// Todos los episodios de una temporada, marcados o desmarcados de golpe. Devuelve los
/// ids afectados para que `server::api` encole la sincronización de cada uno igual que
/// hace el marcado individual (no existe un endpoint de sincronización en lote).
pub fn set_season_watched(conn: &Connection, profile_id: i64, series_id: i64, season: i64, watched: bool) -> Result<Vec<i64>> {
    let episode_ids = episode_ids_for_season(conn, series_id, season)?;
    for &id in &episode_ids {
        set_episode_watched(conn, profile_id, id, watched)?;
    }
    Ok(episode_ids)
}

pub struct RandomPick {
    pub kind: String,
    pub id: i64,
}

/// Título al azar del catálogo entero (película o serie), para el botón "Sorpréndeme"
/// (F "premium"). Cuenta filas y salta a un desplazamiento al azar con `LIMIT 1 OFFSET`
/// en vez de `ORDER BY RANDOM()` (que forzaría ordenar las ~100.000 filas del catálogo,
/// ver R4) — esto es una única búsqueda puntual, no paginación, así que el `OFFSET` no
/// entra en conflicto con la regla de "cursor, nunca offset" del resto de este fichero:
/// esa regla es sobre recorrer el catálogo página a página, no sobre pedir una fila.
/// Prioriza contenido ya emparejado con TMDB (con póster y sinopsis de verdad); si el
/// catálogo todavía no tiene nada emparejado, cae a cualquier título.
pub fn random_title(conn: &Connection, access: ProfileAccess) -> Result<Option<RandomPick>> {
    if let Some(pick) = random_title_matched_only(conn, true, access)? {
        return Ok(Some(pick));
    }
    random_title_matched_only(conn, false, access)
}

/// Al azar entre lo que el perfil puede ver (F14: "Sorpréndeme" no podía saltarse el
/// filtro de un perfil infantil).
fn random_title_matched_only(conn: &Connection, matched_only: bool, access: ProfileAccess) -> Result<Option<RandomPick>> {
    use rand::RngExt;
    let matched = if matched_only { " AND t.tmdb_state = 'matched'" } else { "" };
    let movie_where = format!("WHERE 1=1{matched}{}", title_access_sql(TitleTable::Movies, "t", access));
    let series_where = format!("WHERE 1=1{matched}{}", title_access_sql(TitleTable::Series, "t", access));
    let movie_count: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM movies t {movie_where}"), [], |row| row.get(0))?;
    let series_count: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM series t {series_where}"), [], |row| row.get(0))?;
    let total = movie_count + series_count;
    if total == 0 {
        return Ok(None);
    }
    let roll = rand::rng().random_range(0..total);
    if roll < movie_count {
        let id: i64 = conn.query_row(&format!("SELECT t.id FROM movies t {movie_where} LIMIT 1 OFFSET ?1"), params![roll], |row| row.get(0))?;
        Ok(Some(RandomPick { kind: "movie".to_string(), id }))
    } else {
        let offset = roll - movie_count;
        let id: i64 = conn.query_row(&format!("SELECT t.id FROM series t {series_where} LIMIT 1 OFFSET ?1"), params![offset], |row| row.get(0))?;
        Ok(Some(RandomPick { kind: "series".to_string(), id }))
    }
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
    fn list_genre_rows_picks_the_genres_with_the_most_movies_and_never_unmatched_ones() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        // Género 28 (Acción): tres películas emparejadas. Género 18 (Drama): una.
        for i in 0..3 {
            let id = seed_movie(&conn, provider_id, &format!("Accion {i}"), Some(2020), Some("[28]"), None);
            conn.execute("UPDATE movies SET tmdb_state = 'matched' WHERE id = ?1", params![id]).unwrap();
        }
        let drama_id = seed_movie(&conn, provider_id, "Drama Uno", Some(2020), Some("[18]"), None);
        conn.execute("UPDATE movies SET tmdb_state = 'matched' WHERE id = ?1", params![drama_id]).unwrap();

        // Sin emparejar: no debe contar para ningún género, aunque tenga la etiqueta.
        seed_movie(&conn, provider_id, "Accion Pendiente", Some(2020), Some("[28]"), None);

        let rows = list_genre_rows(&conn, 2, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, 28); // el más numeroso, primero.
        assert_eq!(rows[0].movies.len(), 3);
        assert_eq!(rows[1].id, 18);
        assert_eq!(rows[1].movies.len(), 1);
    }

    #[test]
    fn list_recommendation_seed_movies_includes_favorites_and_mostly_watched_but_not_barely_started() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let profile_id = ensure_default_profile(&conn).unwrap();

        let favorited = seed_movie(&conn, provider_id, "Favorita", Some(2020), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 1 WHERE id = ?1", params![favorited]).unwrap();
        add_favorite(&conn, profile_id, "movie", favorited).unwrap();

        let mostly_watched = seed_movie(&conn, provider_id, "Casi Terminada", Some(2020), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 2 WHERE id = ?1", params![mostly_watched]).unwrap();
        upsert_playback_progress(&conn, profile_id, "movie", mostly_watched, 5400, Some(6000)).unwrap(); // 90%

        let barely_started = seed_movie(&conn, provider_id, "Recién Empezada", Some(2020), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 3 WHERE id = ?1", params![barely_started]).unwrap();
        upsert_playback_progress(&conn, profile_id, "movie", barely_started, 60, Some(6000)).unwrap(); // 1%

        let no_tmdb_id = seed_movie(&conn, provider_id, "Sin TMDB", Some(2020), None, None);
        add_favorite(&conn, profile_id, "movie", no_tmdb_id).unwrap();

        let seeds = list_recommendation_seed_movies(&conn, profile_id, 10).unwrap();
        let titles: Vec<&str> = seeds.iter().map(|s| s.clean_title.as_str()).collect();
        assert!(titles.contains(&"Favorita"));
        assert!(titles.contains(&"Casi Terminada"));
        assert!(!titles.contains(&"Recién Empezada")); // menos del 70%, no cuenta como "le gustó".
        assert!(!titles.contains(&"Sin TMDB")); // sin tmdb_id no hay nada que preguntarle a TMDB.
    }

    #[test]
    fn list_recommendation_seed_series_includes_favorites_and_series_with_a_finished_episode() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let profile_id = ensure_default_profile(&conn).unwrap();

        let (favorited_id, _) = seed_series_with_episodes(&conn, provider_id, "Favorita", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 1 WHERE id = ?1", params![favorited_id]).unwrap();
        add_favorite(&conn, profile_id, "series", favorited_id).unwrap();

        let (finished_id, finished_episodes) = seed_series_with_episodes(&conn, provider_id, "Con Episodio Visto", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 2 WHERE id = ?1", params![finished_id]).unwrap();
        upsert_playback_progress(&conn, profile_id, "episode", finished_episodes[0], 1150, Some(1200)).unwrap(); // >90%.

        let (barely_started_id, barely_started_episodes) = seed_series_with_episodes(&conn, provider_id, "Recién Empezada", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 3 WHERE id = ?1", params![barely_started_id]).unwrap();
        upsert_playback_progress(&conn, profile_id, "episode", barely_started_episodes[0], 60, Some(1200)).unwrap(); // 5%.

        let (no_tmdb_id, _) = seed_series_with_episodes(&conn, provider_id, "Sin TMDB", &[(1, 1)]);
        add_favorite(&conn, profile_id, "series", no_tmdb_id).unwrap();

        let seeds = list_recommendation_seed_series(&conn, profile_id, 10).unwrap();
        let titles: Vec<&str> = seeds.iter().map(|s| s.clean_title.as_str()).collect();
        assert!(titles.contains(&"Favorita"));
        assert!(titles.contains(&"Con Episodio Visto"));
        assert!(!titles.contains(&"Recién Empezada")); // el único episodio no llega al 90%.
        assert!(!titles.contains(&"Sin TMDB"));
    }

    #[test]
    fn resolve_series_recommendations_excludes_seeds_and_deduplicates() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let (already_seen, _) = seed_series_with_episodes(&conn, provider_id, "Ya Vista", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 100 WHERE id = ?1", params![already_seen]).unwrap();
        let (recommended, _) = seed_series_with_episodes(&conn, provider_id, "Nueva Sugerencia", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 200 WHERE id = ?1", params![recommended]).unwrap();

        let candidates = vec![
            (100, "Semilla".to_string()), // ya vista: se excluye aunque TMDB la sugiera.
            (200, "Semilla".to_string()), // sí está en el catálogo local.
            (200, "Semilla".to_string()), // duplicada: solo debe contar una vez.
            (999, "Semilla".to_string()), // no está en el catálogo local: se descarta sola.
        ];

        let results = resolve_series_recommendations(&conn, &candidates, &[already_seen], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].clean_title, "Nueva Sugerencia");
        assert_eq!(results[0].because_of, "Semilla");
    }

    #[test]
    fn resolve_recommendations_excludes_seeds_and_deduplicates() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let already_seen = seed_movie(&conn, provider_id, "Ya Vista", Some(2020), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 100 WHERE id = ?1", params![already_seen]).unwrap();
        let recommended = seed_movie(&conn, provider_id, "Nueva Sugerencia", Some(2021), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 200 WHERE id = ?1", params![recommended]).unwrap();

        let candidates = vec![
            (100, "Semilla".to_string()),  // ya vista: se excluye aunque TMDB la sugiera.
            (200, "Semilla".to_string()),  // sí está en el catálogo local.
            (200, "Semilla".to_string()),  // duplicada: solo debe contar una vez.
            (999, "Semilla".to_string()),  // no está en el catálogo local: se descarta sola.
        ];

        let results = resolve_recommendations(&conn, &candidates, &[already_seen], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].clean_title, "Nueva Sugerencia");
        assert_eq!(results[0].because_of, "Semilla");
    }

    #[test]
    fn resolve_similar_movies_excludes_the_title_itself_and_deduplicates() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let origin = seed_movie(&conn, provider_id, "Origen", Some(2020), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 100 WHERE id = ?1", params![origin]).unwrap();
        let similar = seed_movie(&conn, provider_id, "Parecida", Some(2021), None, None);
        conn.execute("UPDATE movies SET tmdb_id = 200 WHERE id = ?1", params![similar]).unwrap();

        // TMDB puede sugerir la propia película entre sus "similares" (le pasa de
        // verdad, no es un caso de laboratorio) — 200 repetido debe contar una vez, 100
        // (la propia ficha) y 999 (fuera del catálogo local) deben descartarse.
        let results = resolve_similar_movies(&conn, &[100, 200, 200, 999], origin, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].clean_title, "Parecida");
    }

    #[test]
    fn resolve_similar_series_excludes_the_title_itself() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (origin_id, _) = seed_series_with_episodes(&conn, provider_id, "Origen", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 100 WHERE id = ?1", params![origin_id]).unwrap();
        let (similar_id, _) = seed_series_with_episodes(&conn, provider_id, "Parecida", &[(1, 1)]);
        conn.execute("UPDATE series SET tmdb_id = 200 WHERE id = ?1", params![similar_id]).unwrap();

        let results = resolve_similar_series(&conn, &[100, 200], origin_id, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].clean_title, "Parecida");
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
        let series = RawSeries { remote_id: "s1".into(), raw_title: "La Casa de Papel".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "La Casa de Papel", year: None, match_key: "la-casa-de-papel-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series, &fields).unwrap();
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

        let hidden = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, None, 50, ProfileAccess::NO_ADULT).unwrap();
        assert!(hidden.iter().all(|m| m.clean_title != "Peli Adulta"));
        assert!(hidden.iter().any(|m| m.clean_title == "Peli Normal"));

        let unlocked = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, None, 50, ProfileAccess::UNRESTRICTED).unwrap();
        assert!(unlocked.iter().any(|m| m.clean_title == "Peli Adulta"));
    }

    #[test]
    fn list_movies_filters_by_genre_via_json_each() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Accion Uno", Some(2020), Some("[28,12]"), None);
        seed_movie(&conn, provider_id, "Drama Uno", Some(2020), Some("[18]"), None);

        let action = list_movies_page(&conn, Some(28), None, None, MovieSort::AddedDesc, None, None, 50, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(action.len(), 1);
        assert_eq!(action[0].clean_title, "Accion Uno");
    }

    #[test]
    fn list_movies_filters_by_year_and_quality() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Peli 2020 4K", Some(2020), None, Some("4K"));
        seed_movie(&conn, provider_id, "Peli 2021 HD", Some(2021), None, Some("HD"));

        let by_year = list_movies_page(&conn, None, Some(2020), None, MovieSort::AddedDesc, None, None, 50, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(by_year.len(), 1);

        let by_quality = list_movies_page(&conn, None, None, Some("4K"), MovieSort::AddedDesc, None, None, 50, ProfileAccess::UNRESTRICTED).unwrap();
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

        let first_page = list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, None, None, 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(first_page.len(), 2);
        let cursor = first_page.last().unwrap().id;

        let second_page =
            list_movies_page(&conn, None, None, None, MovieSort::AddedDesc, Some(PageCursor { id: cursor, key: None }), None, 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(second_page.len(), 2);
        assert!(second_page.iter().all(|m| m.id != cursor));
    }

    /// El bug real que arregla el cursor compuesto: antes, pedir "después del id X"
    /// ordenando por título no tenía ninguna relación con el orden real, así que la
    /// segunda página podía repetir o saltarse títulos. Con nombres elegidos a propósito
    /// para que el orden alfabético NO coincida con el orden de inserción (ids), esto
    /// habría fallado con la implementación vieja.
    #[test]
    fn list_movies_pagination_by_title_is_exact_even_when_alphabetical_order_differs_from_insertion_order() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        for title in ["Zorro", "Amanecer", "Medianoche", "Beta", "Yugo"] {
            seed_movie(&conn, provider_id, title, Some(2020), None, None);
        }

        let mut seen = Vec::new();
        let mut cursor: Option<PageCursor> = None;
        let mut after_text: Option<String> = None;
        loop {
            let page = list_movies_page(&conn, None, None, None, MovieSort::TitleAsc, cursor, after_text.as_deref(), 2, ProfileAccess::UNRESTRICTED).unwrap();
            if page.is_empty() {
                break;
            }
            for item in &page {
                seen.push(item.clean_title.clone());
            }
            let (next_cursor, next_key) = movie_page_cursor(MovieSort::TitleAsc, page.last().unwrap());
            cursor = Some(next_cursor);
            after_text = next_key;
            if page.len() < 2 {
                break;
            }
        }

        assert_eq!(seen, vec!["Amanecer", "Beta", "Medianoche", "Yugo", "Zorro"]);
    }

    #[test]
    fn list_movies_pagination_by_rating_is_exact() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        for (title, rating) in [("Baja", 3.0), ("Alta", 9.5), ("Media", 6.0)] {
            let id = seed_movie(&conn, provider_id, title, Some(2020), None, None);
            conn.execute("UPDATE movies SET rating = ?1 WHERE id = ?2", params![rating, id]).unwrap();
        }

        let first = list_movies_page(&conn, None, None, None, MovieSort::RatingDesc, None, None, 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(first.iter().map(|m| m.clean_title.as_str()).collect::<Vec<_>>(), vec!["Alta", "Media"]);

        let (cursor, _) = movie_page_cursor(MovieSort::RatingDesc, first.last().unwrap());
        let second = list_movies_page(&conn, None, None, None, MovieSort::RatingDesc, Some(cursor), None, 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(second.iter().map(|m| m.clean_title.as_str()).collect::<Vec<_>>(), vec!["Baja"]);
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

        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Aventura Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Aventura Serie", year: None, match_key: "aventura-serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();

        queries::rebuild_search_index(&conn).unwrap();

        let hits = search(&conn, "Aventura", 10, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.item_kind == "movie"));
        assert!(hits.iter().any(|h| h.item_kind == "series"));
    }

    /// Sin esto, la búsqueda era una lista de texto plano (F "premium"): la portada no
    /// vive en el propio índice FTS5, se busca aparte tabla por tabla.
    #[test]
    fn search_includes_poster_path_from_the_matching_table() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Con Portada", Some(2020), None, None);
        conn.execute("UPDATE movies SET poster_path = '/poster.jpg' WHERE id = ?1", params![movie_id]).unwrap();
        queries::rebuild_search_index(&conn).unwrap();

        let hits = search(&conn, "Portada", 10, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].poster_path.as_deref(), Some("/poster.jpg"));
    }

    #[test]
    fn search_with_garbage_input_returns_empty_not_error() {
        let conn = memory_conn();
        queries::rebuild_search_index(&conn).unwrap();
        let hits = search(&conn, "***", 10, ProfileAccess::UNRESTRICTED).unwrap();
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

    fn watched_ids(conn: &Connection, profile_id: i64, series_id: i64) -> std::collections::HashSet<i64> {
        list_episode_progress(conn, profile_id, series_id).unwrap().into_iter().filter(|(_, p)| p.finished).map(|(id, _)| id).collect()
    }

    fn progress(finished: bool, position_s: i64, played_at: i64) -> EpisodeProgress {
        EpisodeProgress { finished, position_s, played_at }
    }

    #[test]
    fn up_next_starts_at_the_first_episode_without_progress() {
        assert_eq!(up_next_episode(&[10, 11, 12], &HashMap::new()), Some((10, false)));
        assert_eq!(up_next_episode(&[], &HashMap::new()), None);
    }

    #[test]
    fn up_next_resumes_an_episode_left_halfway() {
        let map = HashMap::from([(10, progress(true, 1200, 100)), (11, progress(false, 300, 200))]);
        assert_eq!(up_next_episode(&[10, 11, 12], &map), Some((11, true)));
    }

    #[test]
    fn up_next_goes_to_the_episode_after_the_last_one_finished() {
        let map = HashMap::from([(10, progress(true, 1200, 100))]);
        assert_eq!(up_next_episode(&[10, 11, 12], &map), Some((11, true)));
    }

    #[test]
    fn up_next_follows_what_was_touched_last_not_the_furthest() {
        // Vio el 3 hace tiempo y ahora ha vuelto a ver el 1: sigue por el 2.
        let map = HashMap::from([(12, progress(true, 1200, 100)), (10, progress(true, 1200, 300))]);
        assert_eq!(up_next_episode(&[10, 11, 12], &map), Some((11, true)));
    }

    #[test]
    fn up_next_after_marking_a_whole_season_is_the_next_season() {
        // Marcar la temporada pone a todos el mismo momento: cuenta el último en orden.
        let map = HashMap::from([(10, progress(true, 0, 500)), (11, progress(true, 0, 500))]);
        assert_eq!(up_next_episode(&[10, 11, 20], &map), Some((20, true)));
    }

    #[test]
    fn up_next_starts_over_after_the_last_episode() {
        let map = HashMap::from([(12, progress(true, 1200, 100))]);
        assert_eq!(up_next_episode(&[10, 11, 12], &map), Some((10, false)));
    }

    #[test]
    fn list_watched_episode_ids_only_counts_finished_episodes_of_that_series() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Serie", year: None, match_key: "serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
        let series_id: i64 = conn.query_row("SELECT id FROM series WHERE match_key = 'serie-0000'", [], |r| r.get(0)).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources WHERE series_id = ?1", params![series_id], |r| r.get(0)).unwrap();

        let episodes = vec![
            crate::sources::model::RawEpisode { remote_id: "e1".into(), season: 1, number: 1, title: None, container_extension: None, duration_s: Some(1200), still_url: None },
            crate::sources::model::RawEpisode { remote_id: "e2".into(), season: 1, number: 2, title: None, container_extension: None, duration_s: Some(1200), still_url: None },
        ];
        upsert_episodes(&conn, series_source_id, &episodes).unwrap();
        let ep1: i64 = conn.query_row("SELECT id FROM episodes WHERE remote_id = 'e1'", [], |r| r.get(0)).unwrap();
        let ep2: i64 = conn.query_row("SELECT id FROM episodes WHERE remote_id = 'e2'", [], |r| r.get(0)).unwrap();

        upsert_playback_progress(&conn, profile_id, "episode", ep1, 1150, Some(1200)).unwrap(); // >90%: visto.
        upsert_playback_progress(&conn, profile_id, "episode", ep2, 60, Some(1200)).unwrap(); // recién empezado: no.

        let watched = watched_ids(&conn, profile_id, series_id);
        assert!(watched.contains(&ep1));
        assert!(!watched.contains(&ep2));
        let all = list_episode_progress(&conn, profile_id, series_id).unwrap();
        assert_eq!(all.get(&ep2).map(|p| (p.finished, p.position_s)), Some((false, 60)));
    }

    #[test]
    fn set_episode_watched_marks_and_unmarks_without_having_played_it() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Serie", year: None, match_key: "serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources WHERE remote_id = '1'", [], |r| r.get(0)).unwrap();
        upsert_episodes(&conn, series_source_id, &[crate::sources::model::RawEpisode { remote_id: "e1".into(), season: 1, number: 1, title: None, container_extension: None, duration_s: Some(1500), still_url: None }]).unwrap();
        let episode_id: i64 = conn.query_row("SELECT id FROM episodes WHERE remote_id = 'e1'", [], |r| r.get(0)).unwrap();

        assert!(get_playback_progress(&conn, profile_id, "episode", episode_id).unwrap().is_none());

        set_episode_watched(&conn, profile_id, episode_id, true).unwrap();
        let saved = get_playback_progress(&conn, profile_id, "episode", episode_id).unwrap().unwrap();
        assert!(saved.finished);
        assert_eq!(saved.duration_s, Some(1500));

        set_episode_watched(&conn, profile_id, episode_id, false).unwrap();
        assert!(get_playback_progress(&conn, profile_id, "episode", episode_id).unwrap().is_none());
    }

    #[test]
    fn set_movie_watched_uses_tmdb_runtime_when_available() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Peli", Some(2020), None, None);
        conn.execute("UPDATE movies SET runtime_min = 100 WHERE id = ?1", params![movie_id]).unwrap();

        set_movie_watched(&conn, profile_id, movie_id, true).unwrap();
        let saved = get_playback_progress(&conn, profile_id, "movie", movie_id).unwrap().unwrap();
        assert!(saved.finished);
        assert_eq!(saved.duration_s, Some(6000)); // 100 min * 60.

        set_movie_watched(&conn, profile_id, movie_id, false).unwrap();
        assert!(get_playback_progress(&conn, profile_id, "movie", movie_id).unwrap().is_none());
    }

    #[test]
    fn set_movie_watched_falls_back_to_1_1_without_any_known_duration() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Peli Sin Duracion", Some(2020), None, None);

        set_movie_watched(&conn, profile_id, movie_id, true).unwrap();
        let saved = get_playback_progress(&conn, profile_id, "movie", movie_id).unwrap().unwrap();
        assert!(saved.finished);
        assert_eq!(saved.duration_s, Some(1));
    }

    #[test]
    fn set_season_watched_marks_every_episode_of_that_season_and_no_other() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Serie", year: None, match_key: "serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
        let series_id: i64 = conn.query_row("SELECT id FROM series WHERE match_key = 'serie-0000'", [], |r| r.get(0)).unwrap();
        let series_source_id: i64 = conn.query_row("SELECT id FROM series_sources WHERE series_id = ?1", params![series_id], |r| r.get(0)).unwrap();
        upsert_episodes(
            &conn,
            series_source_id,
            &[
                crate::sources::model::RawEpisode { remote_id: "s1e1".into(), season: 1, number: 1, title: None, container_extension: None, duration_s: Some(1200), still_url: None },
                crate::sources::model::RawEpisode { remote_id: "s1e2".into(), season: 1, number: 2, title: None, container_extension: None, duration_s: Some(1200), still_url: None },
                crate::sources::model::RawEpisode { remote_id: "s2e1".into(), season: 2, number: 1, title: None, container_extension: None, duration_s: Some(1200), still_url: None },
            ],
        )
        .unwrap();

        let affected = set_season_watched(&conn, profile_id, series_id, 1, true).unwrap();
        assert_eq!(affected.len(), 2);

        let watched = watched_ids(&conn, profile_id, series_id);
        assert_eq!(watched.len(), 2, "solo la temporada 1 debe quedar marcada");
        let s2e1: i64 = conn.query_row("SELECT id FROM episodes WHERE remote_id = 's2e1'", [], |r| r.get(0)).unwrap();
        assert!(!watched.contains(&s2e1));

        set_season_watched(&conn, profile_id, series_id, 1, false).unwrap();
        assert!(watched_ids(&conn, profile_id, series_id).is_empty());
    }

    #[test]
    fn random_title_returns_none_for_an_empty_catalog() {
        let conn = memory_conn();
        assert!(random_title(&conn, ProfileAccess::UNRESTRICTED).unwrap().is_none());
    }

    #[test]
    fn random_title_never_picks_an_unmatched_title_while_a_matched_one_exists() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        seed_movie(&conn, provider_id, "Sin Emparejar", Some(2020), None, None);
        let matched = seed_movie(&conn, provider_id, "Emparejada", Some(2021), None, None);
        conn.execute("UPDATE movies SET tmdb_state = 'matched' WHERE id = ?1", params![matched]).unwrap();

        for _ in 0..20 {
            let pick = random_title(&conn, ProfileAccess::UNRESTRICTED).unwrap().unwrap();
            assert_eq!((pick.kind.as_str(), pick.id), ("movie", matched));
        }
    }

    #[test]
    fn random_title_falls_back_to_any_state_when_nothing_is_matched_yet() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie_id = seed_movie(&conn, provider_id, "Recien Sincronizada", Some(2020), None, None);

        let pick = random_title(&conn, ProfileAccess::UNRESTRICTED).unwrap().unwrap();
        assert_eq!((pick.kind.as_str(), pick.id), ("movie", movie_id));
    }

    #[test]
    fn random_title_can_land_on_a_series() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let (series_id, _) = seed_series_with_episodes(&conn, provider_id, "Serie Unica", &[(1, 1)]);

        let pick = random_title(&conn, ProfileAccess::UNRESTRICTED).unwrap().unwrap();
        assert_eq!((pick.kind.as_str(), pick.id), ("series", series_id));
    }

    #[test]
    fn remove_playback_progress_deletes_the_row_entirely() {
        let conn = memory_conn();
        let profile_id = ensure_default_profile(&conn).unwrap();
        upsert_playback_progress(&conn, profile_id, "movie", 1, 85, Some(100)).unwrap();
        assert!(get_playback_progress(&conn, profile_id, "movie", 1).unwrap().is_some());

        remove_playback_progress(&conn, profile_id, "movie", 1).unwrap();
        assert!(get_playback_progress(&conn, profile_id, "movie", 1).unwrap().is_none());
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
        let series_raw = RawSeries { remote_id: "1".into(), raw_title: "Mi Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Mi Serie", year: None, match_key: "mi-serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
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
        let series_raw = RawSeries { remote_id: title.into(), raw_title: title.into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let match_key = format!("{}-0000", title.to_lowercase().replace(' ', "-"));
        let fields = TitleFields { clean_title: title, year: None, match_key: &match_key, quality: None, language: None };
        queries::upsert_series(conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
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

    /// Mismo criterio que canales/películas, pero para series (migración 005): antes de
    /// esto `series_sources` no tenía categoría, así que este filtro no podía existir.
    #[test]
    fn list_series_page_hides_adult_category_series_unless_unlocked() {
        use crate::sources::model::{MediaKind, RawCategory, RawSeries};
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let map = queries::upsert_categories(
            &conn,
            provider_id,
            MediaKind::Series,
            &[RawCategory { remote_id: "1".into(), kind: MediaKind::Series, name: "XXX".into() }],
        )
        .unwrap();
        conn.execute("UPDATE categories SET is_adult = 1 WHERE id = ?1", params![map["1"]]).unwrap();

        let series = RawSeries {
            remote_id: "1".into(),
            raw_title: "Serie Adulta".into(),
            category_remote_id: Some("1".into()),
            cover_url: None,
            plot: None,
            release_date: None,
        };
        let fields = TitleFields { clean_title: "Serie Adulta", year: None, match_key: "serie-adulta-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &map, &series, &fields).unwrap();

        assert!(list_series_page(&conn, None, None, SeriesSort::AddedDesc, None, None, 50, ProfileAccess::NO_ADULT).unwrap().is_empty());
        assert_eq!(list_series_page(&conn, None, None, SeriesSort::AddedDesc, None, None, 50, ProfileAccess::UNRESTRICTED).unwrap().len(), 1);
    }

    /// Series sincronizadas antes de la migración 005 (o cuyo proveedor no da
    /// categoría) tienen `category_id` a NULL — deben seguir viéndose, no ocultarse
    /// "por si acaso": mejor pecar de enseñar de más en un caso ambiguo que romper el
    /// catálogo entero de quien todavía no ha resincronizado.
    #[test]
    fn list_series_page_never_hides_series_with_no_category_at_all() {
        use crate::sources::model::RawSeries;
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let series = RawSeries {
            remote_id: "1".into(),
            raw_title: "Serie Sin Categoría".into(),
            category_remote_id: None,
            cover_url: None,
            plot: None,
            release_date: None,
        };
        let fields = TitleFields { clean_title: "Serie Sin Categoría", year: None, match_key: "serie-sin-categoria-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series, &fields).unwrap();

        assert_eq!(list_series_page(&conn, None, None, SeriesSort::AddedDesc, None, None, 50, ProfileAccess::NO_ADULT).unwrap().len(), 1);
    }

    /// Mismo bug de fondo que en películas: sin cursor compuesto, ordenar series por
    /// título con una segunda página repetiría o saltaría resultados.
    #[test]
    fn list_series_pagination_by_title_is_exact() {
        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        for (i, title) in ["Zorro", "Amanecer", "Medianoche"].into_iter().enumerate() {
            let series = crate::sources::model::RawSeries {
                remote_id: i.to_string(),
                raw_title: title.into(),
                category_remote_id: None,
                cover_url: None,
                plot: None,
                release_date: None,
            };
            let fields = TitleFields { clean_title: title, year: None, match_key: &format!("serie-{i}"), quality: None, language: None };
            queries::upsert_series(&conn, provider_id, &HashMap::new(), &series, &fields).unwrap();
        }

        let first = list_series_page(&conn, None, None, SeriesSort::TitleAsc, None, None, 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(first.iter().map(|s| s.clean_title.as_str()).collect::<Vec<_>>(), vec!["Amanecer", "Medianoche"]);

        let (cursor, key) = series_page_cursor(SeriesSort::TitleAsc, first.last().unwrap());
        let second = list_series_page(&conn, None, None, SeriesSort::TitleAsc, Some(cursor), key.as_deref(), 2, ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(second.iter().map(|s| s.clean_title.as_str()).collect::<Vec<_>>(), vec!["Zorro"]);
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
        let series_raw = RawSeries { remote_id: "9001".into(), raw_title: "Serie".into(), category_remote_id: None, cover_url: None, plot: None, release_date: None };
        let fields = TitleFields { clean_title: "Serie", year: None, match_key: "serie-0000", quality: None, language: None };
        queries::upsert_series(&conn, provider_id, &HashMap::new(), &series_raw, &fields).unwrap();
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

// ─────────────── Funciones de tele del directo (F13) ───────────────

/// Columnas que espera `channel_list_item`, en este orden.
const CHANNEL_LIST_COLUMNS: &str = "c.id, c.display_name, c.logo_url, c.category_id, c.has_catchup, c.sort_order";

/// Filtro de adultos sobre `channels c`, igual que el de `list_channels_page`.
const NOT_ADULT_CHANNEL: &str = " AND (c.category_id IS NULL OR c.category_id NOT IN (SELECT id FROM categories WHERE is_adult = 1))";

fn channel_list_item(row: &rusqlite::Row) -> rusqlite::Result<ChannelListItem> {
    let raw_logo: Option<String> = row.get(2)?;
    Ok(ChannelListItem {
        id: row.get(0)?,
        display_name: row.get(1)?,
        logo_url: raw_logo.map(|url| redact(&url)),
        category_id: row.get(3)?,
        has_catchup: row.get::<_, i64>(4)? != 0,
        number: row.get(5)?,
    })
}

/// El canal con ese número del proveedor (`sort_order`, el `num` de Xtream). Con varios
/// proveedores el mismo número se repite: gana el del canal que se está viendo (`near`).
/// Si hay varios feeds del mismo canal, el primero.
pub fn channel_by_number(conn: &Connection, number: i64, near: Option<i64>, include_adult: bool) -> Result<Option<ChannelListItem>> {
    let mut sql = format!("SELECT {CHANNEL_LIST_COLUMNS} FROM channels c WHERE c.sort_order = ?1");
    if !include_adult {
        sql.push_str(NOT_ADULT_CHANNEL);
    }
    sql.push_str(" ORDER BY c.provider_id IS NOT (SELECT provider_id FROM channels WHERE id = ?2), c.id LIMIT 1");
    conn.query_row(&sql, params![number, near], channel_list_item).optional().map_err(Into::into)
}

/// Últimos canales que abrió el perfil (cada apertura deja `played_at` en
/// `playback_state`, ver `queries::record_play_tier`).
pub fn recent_channels(conn: &Connection, profile_id: i64, include_adult: bool, limit: i64) -> Result<Vec<ChannelListItem>> {
    let mut sql = format!(
        "SELECT {CHANNEL_LIST_COLUMNS} FROM playback_state ps JOIN channels c ON c.id = ps.item_id
         WHERE ps.profile_id = ?1 AND ps.item_kind = 'channel'"
    );
    if !include_adult {
        sql.push_str(NOT_ADULT_CHANNEL);
    }
    sql.push_str(" ORDER BY ps.played_at DESC LIMIT ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id, limit], channel_list_item)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct EpgSearchRow {
    pub channel: ChannelListItem,
    pub title: String,
    pub subtitle: Option<String>,
    pub start_ts: i64,
    pub stop_ts: i64,
}

/// Programas en emisión o futuros cuyo título contiene `q` (sin distinguir mayúsculas),
/// por orden de comienzo. Un mismo programa sale una vez por canal que lo emite.
pub fn epg_search(conn: &Connection, q: &str, now: i64, include_adult: bool, limit: i64) -> Result<Vec<EpgSearchRow>> {
    let mut sql = format!(
        "SELECT {CHANNEL_LIST_COLUMNS}, p.title, p.subtitle, p.start_ts, p.stop_ts
         FROM epg_programmes p JOIN channels c ON c.epg_channel_id = p.channel_id AND (p.provider_id IS NULL OR p.provider_id = c.provider_id)
         WHERE p.stop_ts > ?1 AND p.title LIKE ?2 ESCAPE '\\'"
    );
    if !include_adult {
        sql.push_str(NOT_ADULT_CHANNEL);
    }
    sql.push_str(" ORDER BY p.start_ts, c.sort_order IS NULL, c.sort_order, c.id LIMIT ?3");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now, like_pattern(q), limit], |row| {
        Ok(EpgSearchRow {
            channel: channel_list_item(row)?,
            title: row.get(6)?,
            subtitle: row.get(7)?,
            start_ts: row.get(8)?,
            stop_ts: row.get(9)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub struct ReminderRow {
    pub channel: ChannelListItem,
    pub title: String,
    pub start_ts: i64,
    pub stop_ts: i64,
}

/// Recordatorios del perfil que aún no han terminado, el más próximo primero. Borra los
/// ya terminados.
pub fn list_reminders(conn: &Connection, profile_id: i64, now: i64) -> Result<Vec<ReminderRow>> {
    conn.execute("DELETE FROM reminders WHERE stop_ts <= ?1", params![now])?;
    let sql = format!(
        "SELECT {CHANNEL_LIST_COLUMNS}, r.title, r.start_ts, r.stop_ts
         FROM reminders r JOIN channels c ON c.id = r.channel_id
         WHERE r.profile_id = ?1 ORDER BY r.start_ts"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id], |row| {
        Ok(ReminderRow { channel: channel_list_item(row)?, title: row.get(6)?, start_ts: row.get(7)?, stop_ts: row.get(8)? })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn add_reminder(conn: &Connection, profile_id: i64, channel_id: i64, start_ts: i64, stop_ts: i64, title: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO reminders (profile_id, channel_id, start_ts, stop_ts, title, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(profile_id, channel_id, start_ts) DO UPDATE SET stop_ts = excluded.stop_ts, title = excluded.title",
        params![profile_id, channel_id, start_ts, stop_ts, title, crate::db::now_epoch()],
    )?;
    Ok(())
}

pub fn delete_reminder(conn: &Connection, profile_id: i64, channel_id: i64, start_ts: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM reminders WHERE profile_id = ?1 AND channel_id = ?2 AND start_ts = ?3",
        params![profile_id, channel_id, start_ts],
    )?;
    Ok(())
}
