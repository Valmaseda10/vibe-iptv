//! Endpoints HTTP del catálogo (F4). Todavía solo en 127.0.0.1. Ninguna respuesta lleva
//! credenciales ni URLs de proveedor: los streams se piden por `/stream/{kind}/{id}`
//! (F6), nunca se devuelve la URL real de un ítem aquí.

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::tmdb::{TmdbClient, TmdbExtras, TMDB_API_KEY_CREDENTIAL_REF};
use crate::db::reads::{self, MovieSort};
use crate::db::{queries, schema, DbError};
use crate::sources::{Source, VodInfo};
use crate::util::keychain;

#[derive(Clone)]
pub struct CatalogState {
    pub db_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("no encontrado")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Db(_) | ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| ApiError::Internal)?
    .map_err(ApiError::from)
}

// ─────────────── Tipos de respuesta (ts-rs) ───────────────

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct CategorySummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
}

impl From<reads::CategoryRow> for CategorySummary {
    fn from(row: reads::CategoryRow) -> Self {
        Self { id: row.id, name: row.name }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ChannelSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub display_name: String,
    pub logo_url: Option<String>,
    #[ts(type = "number | null")]
    pub category_id: Option<i64>,
    pub has_catchup: bool,
}

impl From<reads::ChannelListItem> for ChannelSummary {
    fn from(row: reads::ChannelListItem) -> Self {
        Self { id: row.id, display_name: row.display_name, logo_url: row.logo_url, category_id: row.category_id, has_catchup: row.has_catchup }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ChannelsPage {
    pub items: Vec<ChannelSummary>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct MovieSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub tmdb_state: String,
    /// Añadida al catálogo en los últimos 7 días (F "premium") — para la insignia
    /// "Nuevo" de las tarjetas. Se calcula aquí, no se manda `added_at` en crudo: el
    /// umbral es cosa del servidor, no algo que cada pantalla del frontend deba repetir.
    pub is_new: bool,
}

const NEW_BADGE_WINDOW_SECONDS: i64 = 7 * 24 * 3600;

impl From<reads::MovieListItem> for MovieSummary {
    fn from(row: reads::MovieListItem) -> Self {
        let is_new = row.added_at.is_some_and(|t| crate::db::now_epoch() - t < NEW_BADGE_WINDOW_SECONDS);
        Self { id: row.id, clean_title: row.clean_title, year: row.year, poster_path: row.poster_path, rating: row.rating, tmdb_state: row.tmdb_state, is_new }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct MoviesPage {
    pub items: Vec<MovieSummary>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
    /// Solo presente cuando el orden no es "recientes" — ver el comentario de cabecera
    /// de `db::reads` sobre paginación por cursor compuesto. El cliente solo tiene que
    /// devolvérselo tal cual a la página siguiente, nunca interpretarlo.
    pub next_cursor_key: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct MovieSourceSummary {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub provider_id: i64,
    pub remote_id: String,
    pub quality: Option<String>,
    pub language: Option<String>,
    pub container: Option<String>,
    /// Duración/códecs reales del fichero (F "premium", `Source::vod_info`) — `None`
    /// hasta que se comprueba al abrir la ficha, o si el proveedor no dio nada.
    #[ts(type = "number | null")]
    pub duration_s: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

impl From<reads::MovieSourceItem> for MovieSourceSummary {
    fn from(row: reads::MovieSourceItem) -> Self {
        Self {
            id: row.id,
            provider_id: row.provider_id,
            remote_id: row.remote_id,
            quality: row.quality,
            language: row.language,
            container: row.container,
            duration_s: row.duration_s,
            video_codec: row.video_codec,
            audio_codec: row.audio_codec,
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct MovieDetailResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    pub runtime_min: Option<i32>,
    #[ts(type = "number[]")]
    pub genres: Vec<i64>,
    pub tmdb_state: String,
    /// Clave de YouTube del tráiler, si hay uno — `None` tanto si no hay como si
    /// todavía no se ha comprobado (la ficha lo comprueba sola al abrirse).
    pub trailer_key: Option<String>,
    /// Certificación por edad de TMDB (p. ej. "16", "PG-13") — `None` si TMDB no tiene
    /// clasificación para esto, o si todavía no se ha comprobado. Nunca del proveedor.
    pub age_rating: Option<String>,
    pub sources: Vec<MovieSourceSummary>,
}

impl From<reads::MovieDetail> for MovieDetailResponse {
    fn from(row: reads::MovieDetail) -> Self {
        Self {
            id: row.id,
            clean_title: row.clean_title,
            year: row.year,
            overview: row.overview,
            poster_path: row.poster_path,
            backdrop_path: row.backdrop_path,
            rating: row.rating,
            runtime_min: row.runtime_min,
            genres: row.genres,
            tmdb_state: row.tmdb_state,
            trailer_key: row.youtube_trailer_key,
            age_rating: row.age_rating,
            sources: row.sources.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SeriesSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub tmdb_state: String,
}

impl From<reads::SeriesListItem> for SeriesSummary {
    fn from(row: reads::SeriesListItem) -> Self {
        Self { id: row.id, clean_title: row.clean_title, year: row.year, poster_path: row.poster_path, rating: row.rating, tmdb_state: row.tmdb_state }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SeriesPage {
    pub items: Vec<SeriesSummary>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
    pub next_cursor_key: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct EpisodeSummary {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub series_source_id: i64,
    #[ts(type = "number")]
    pub season: i64,
    #[ts(type = "number")]
    pub number: i64,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    #[ts(type = "number | null")]
    pub duration_s: Option<i64>,
    /// Checkmark de "visto" (F "premium") — se rellena aparte en `get_series_detail`, no
    /// aquí: este `From` no conoce el perfil que está pidiendo la ficha.
    pub watched: bool,
}

impl From<reads::EpisodeItem> for EpisodeSummary {
    fn from(row: reads::EpisodeItem) -> Self {
        Self { id: row.id, series_source_id: row.series_source_id, season: row.season, number: row.number, title: row.title, overview: row.overview, still_path: row.still_path, duration_s: row.duration_s, watched: false }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SeriesDetailResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    #[ts(type = "number[]")]
    pub genres: Vec<i64>,
    pub tmdb_state: String,
    pub trailer_key: Option<String>,
    pub age_rating: Option<String>,
    pub episodes: Vec<EpisodeSummary>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SearchHitResponse {
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
    pub title: String,
    pub poster_path: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SearchResults {
    pub channels: Vec<SearchHitResponse>,
    pub movies: Vec<SearchHitResponse>,
    pub series: Vec<SearchHitResponse>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ContinueWatchingItem {
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
    #[ts(type = "number")]
    pub position_s: i64,
    #[ts(type = "number | null")]
    pub duration_s: Option<i64>,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster_path: Option<String>,
}

impl From<reads::ContinueWatchingRow> for ContinueWatchingItem {
    fn from(row: reads::ContinueWatchingRow) -> Self {
        Self {
            item_kind: row.item_kind,
            item_id: row.item_id,
            position_s: row.position_s,
            duration_s: row.duration_s,
            title: row.title,
            subtitle: row.subtitle,
            poster_path: row.poster_path,
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct NextEpisodeItem {
    #[ts(type = "number")]
    pub series_id: i64,
    pub series_title: String,
    pub poster_path: Option<String>,
    #[ts(type = "number")]
    pub episode_id: i64,
    #[ts(type = "number")]
    pub season: i64,
    #[ts(type = "number")]
    pub number: i64,
    pub episode_title: Option<String>,
    pub still_path: Option<String>,
}

impl From<reads::NextEpisodeRow> for NextEpisodeItem {
    fn from(row: reads::NextEpisodeRow) -> Self {
        Self {
            series_id: row.series_id,
            series_title: row.series_title,
            poster_path: row.poster_path,
            episode_id: row.episode_id,
            season: row.season,
            number: row.number,
            episode_title: row.episode_title,
            still_path: row.still_path,
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct FeaturedItem {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
}

impl From<reads::FeaturedMovieRow> for FeaturedItem {
    fn from(row: reads::FeaturedMovieRow) -> Self {
        Self {
            id: row.id,
            clean_title: row.clean_title,
            year: row.year,
            overview: row.overview,
            poster_path: row.poster_path,
            backdrop_path: row.backdrop_path,
            rating: row.rating,
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct HomeResponse {
    /// `None` cuando el catálogo todavía no tiene ninguna candidata decente (recién
    /// sincronizado, o nada con fondo+sinopsis de TMDB todavía) — el banner
    /// simplemente no se pinta en ese caso, en vez de enseñar uno vacío.
    pub featured: Option<FeaturedItem>,
    pub continue_watching: Vec<ContinueWatchingItem>,
    pub next_episodes: Vec<NextEpisodeItem>,
    pub recent_movies: Vec<MovieSummary>,
    /// "Recomendado para ti" (F "premium") — vacío mientras no haya nada favorito/visto
    /// que sirva de semilla, o sin clave de TMDB configurada. Nunca bloquea la
    /// respuesta: ver `catalog::recommendations`.
    pub recommended: Vec<RecommendedMovieSummary>,
    /// Filas por género (F "premium"): los géneros con más películas en el catálogo de
    /// verdad del usuario, no una lista fija. Vacío hasta la primera sincronización con
    /// TMDB (sin nombres de género que enseñar, ver `catalog::tmdb::sync_genre_names`).
    pub genre_rows: Vec<GenreRowResponse>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct RecommendedMovieSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
}

impl From<crate::catalog::recommendations::RecommendedMovie> for RecommendedMovieSummary {
    fn from(m: crate::catalog::recommendations::RecommendedMovie) -> Self {
        Self { id: m.id, clean_title: m.clean_title, year: m.year, poster_path: m.poster_path, rating: m.rating, because_of: m.because_of }
    }
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PlaybackProgressInput {
    #[ts(type = "number")]
    pub profile_id: i64,
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
    #[ts(type = "number")]
    pub position_s: i64,
    #[ts(type = "number | null")]
    pub duration_s: Option<i64>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct FavoriteItem {
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
    pub title: String,
    pub poster_path: Option<String>,
}

impl From<reads::FavoriteRow> for FavoriteItem {
    fn from(row: reads::FavoriteRow) -> Self {
        Self { item_kind: row.item_kind, item_id: row.item_id, title: row.title, poster_path: row.poster_path }
    }
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct FavoriteInput {
    #[ts(type = "number")]
    pub profile_id: i64,
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
}

// ─────────────── Handlers ───────────────

const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 200;

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

#[derive(Deserialize)]
struct ProfileQuery {
    profile_id: i64,
}

async fn get_home(State(state): State<CatalogState>, Query(query): Query<ProfileQuery>) -> Result<Json<HomeResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let profile_id = query.profile_id;
    let (featured, continue_watching, next_episodes, recent_movies, genre_rows) = with_db(db_path.clone(), move |conn| {
        let featured = reads::get_featured_movie(conn)?;
        let continue_watching = reads::list_continue_watching(conn, profile_id, 20)?;
        let next_episodes = reads::list_next_episodes(conn, profile_id, 20)?;
        let recent_movies = reads::list_recent_movies(conn, 20)?;
        let genre_rows = reads::list_genre_rows(conn, 3, 20)?;
        let genre_names = reads::list_genre_names(conn, "movie")?;
        Ok((featured, continue_watching, next_episodes, recent_movies, (genre_rows, genre_names)))
    })
    .await?;
    let (genre_rows, genre_names) = genre_rows;
    let genre_name_by_id: std::collections::HashMap<i64, String> = genre_names.into_iter().map(|g| (g.id, g.name)).collect();

    // Nunca bloquea la respuesta (puede implicar varias llamadas a TMDB la primera vez):
    // ver el comentario de cabecera de `catalog::recommendations`.
    let recommended = crate::catalog::recommendations::get_cached_or_refresh(db_path, profile_id).await;

    Ok(Json(HomeResponse {
        featured: featured.map(Into::into),
        continue_watching: continue_watching.into_iter().map(Into::into).collect(),
        next_episodes: next_episodes.into_iter().map(Into::into).collect(),
        recent_movies: recent_movies.into_iter().map(Into::into).collect(),
        recommended: recommended.into_iter().map(Into::into).collect(),
        genre_rows: genre_rows
            .into_iter()
            // Sin nombre todavía (no ha corrido nunca el emparejamiento con TMDB): mejor
            // no enseñar la fila que enseñar "28" como si fuera un título de género.
            .filter_map(|row| {
                let name = genre_name_by_id.get(&row.id)?.clone();
                Some(GenreRowResponse { name, movies: row.movies.into_iter().map(Into::into).collect() })
            })
            .collect(),
    }))
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct GenreRowResponse {
    pub name: String,
    pub movies: Vec<MovieSummary>,
}

#[derive(Deserialize)]
struct LiveCategoriesQuery {
    profile_id: i64,
}

async fn get_live_categories(State(state): State<CatalogState>, Query(query): Query<LiveCategoriesQuery>) -> Result<Json<Vec<CategorySummary>>, ApiError> {
    let items = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::list_categories(conn, "live", include_adult)
    })
    .await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct ChannelsQuery {
    profile_id: i64,
    category_id: Option<i64>,
    q: Option<String>,
    after: Option<i64>,
    limit: Option<i64>,
}

async fn get_live_channels(State(state): State<CatalogState>, Query(query): Query<ChannelsQuery>) -> Result<Json<ChannelsPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let items = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::list_channels_page(conn, query.category_id, query.q.as_deref(), query.after, limit, include_adult)
    })
    .await?;
    let next_cursor = if items.len() as i64 == limit { items.last().map(|c| c.id) } else { None };
    Ok(Json(ChannelsPage { items: items.into_iter().map(Into::into).collect(), next_cursor }))
}

#[derive(Deserialize)]
struct MoviesQuery {
    profile_id: i64,
    genre: Option<i64>,
    year: Option<i32>,
    quality: Option<String>,
    sort: Option<String>,
    after: Option<i64>,
    /// Ver `MoviesPage.next_cursor_key`: se manda tal cual de vuelta, sin que el cliente
    /// necesite saber qué significa para cada orden.
    after_key: Option<String>,
    limit: Option<i64>,
}

fn parse_movie_sort(sort: Option<&str>) -> MovieSort {
    match sort {
        Some("title") => MovieSort::TitleAsc,
        Some("year") => MovieSort::YearDesc,
        Some("rating") => MovieSort::RatingDesc,
        _ => MovieSort::AddedDesc,
    }
}

/// Reconstruye el `PageCursor` tipado a partir de los dos parámetros sueltos de la URL
/// (`after`, `after_key`) — el tipo de la clave depende del orden pedido, no de lo que
/// venga en la propia cadena.
fn parse_page_cursor(sort_key: Option<SortKeyKind>, after: Option<i64>, after_key: Option<&str>) -> Option<reads::PageCursor> {
    let id = after?;
    let key = match sort_key {
        Some(SortKeyKind::Text) => Some(reads::CursorKey::Text),
        Some(SortKeyKind::Int) => after_key.and_then(|k| k.parse::<i32>().ok()).map(reads::CursorKey::Int),
        Some(SortKeyKind::Float) => after_key.and_then(|k| k.parse::<f64>().ok()).map(reads::CursorKey::Float),
        None => None,
    };
    Some(reads::PageCursor { id, key })
}

/// Qué *tipo* de clave le corresponde a cada orden — no el valor en sí, que llega en la
/// petición. `None` es "recientes" (sin clave, solo `id`).
#[derive(Clone, Copy)]
enum SortKeyKind {
    Text,
    Int,
    Float,
}

fn movie_sort_cursor_key(sort: MovieSort) -> Option<SortKeyKind> {
    match sort {
        MovieSort::AddedDesc => None,
        MovieSort::TitleAsc => Some(SortKeyKind::Text),
        MovieSort::YearDesc => Some(SortKeyKind::Int),
        MovieSort::RatingDesc => Some(SortKeyKind::Float),
    }
}

fn series_sort_cursor_key(sort: reads::SeriesSort) -> Option<SortKeyKind> {
    match sort {
        reads::SeriesSort::AddedDesc => None,
        reads::SeriesSort::TitleAsc => Some(SortKeyKind::Text),
        reads::SeriesSort::YearDesc => Some(SortKeyKind::Int),
        reads::SeriesSort::RatingDesc => Some(SortKeyKind::Float),
    }
}

fn parse_series_sort(sort: Option<&str>) -> reads::SeriesSort {
    match sort {
        Some("title") => reads::SeriesSort::TitleAsc,
        Some("year") => reads::SeriesSort::YearDesc,
        Some("rating") => reads::SeriesSort::RatingDesc,
        _ => reads::SeriesSort::AddedDesc,
    }
}

async fn get_movies(State(state): State<CatalogState>, Query(query): Query<MoviesQuery>) -> Result<Json<MoviesPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let sort = parse_movie_sort(query.sort.as_deref());
    let cursor = parse_page_cursor(movie_sort_cursor_key(sort), query.after, query.after_key.as_deref());
    let after_key = query.after_key.clone();
    let items = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::list_movies_page(conn, query.genre, query.year, query.quality.as_deref(), sort, cursor, after_key.as_deref(), limit, include_adult)
    })
    .await?;
    let (next_cursor, next_cursor_key) = if items.len() as i64 == limit {
        match items.last() {
            Some(last) => {
                let (cursor, key) = reads::movie_page_cursor(sort, last);
                (Some(cursor.id), key)
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    Ok(Json(MoviesPage { items: items.into_iter().map(Into::into).collect(), next_cursor, next_cursor_key }))
}

/// Ficha de película: si el tráiler no se ha comprobado nunca (`trailer_checked_at` a
/// NULL) y hay `tmdb_id`, se comprueba antes de responder — una sola vez por película en
/// toda su vida (regla R4: con hasta 100.000 títulos, nunca durante la sincronización
/// masiva). Sin clave de TMDB configurada, o si la comprobación falla, se responde igual
/// con lo que ya hubiera en caché: no es motivo para romper la ficha entera.
async fn get_movie_detail(State(state): State<CatalogState>, AxumPath(id): AxumPath<i64>) -> Result<Json<MovieDetailResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let movie = with_db(db_path.clone(), move |conn| reads::get_movie(conn, id)).await?;
    let mut movie = movie.ok_or(ApiError::NotFound)?;

    if movie.trailer_checked_at.is_none() {
        if let Some(tmdb_id) = movie.tmdb_id {
            match fetch_and_store_movie_extras(db_path.clone(), id, tmdb_id).await {
                Ok(extras) => {
                    movie.youtube_trailer_key = extras.trailer_key;
                    movie.age_rating = extras.age_rating;
                }
                Err(err) => tracing::warn!(movie_id = id, error = %err, "no se pudo comprobar el tráiler/clasificación"),
            }
        }
    }

    // Duración/códecs reales (F "premium", `Source::vod_info`): solo la primera fuente
    // sin comprobar, y solo una por visita a la ficha — no tiene sentido preguntarle al
    // proveedor por cada fuente disponible de la misma película a la vez.
    if let Some(source) = movie.sources.iter().find(|s| s.vod_info_checked_at.is_none()) {
        let (source_id, provider_id, remote_id) = (source.id, source.provider_id, source.remote_id.clone());
        match fetch_and_store_vod_info(db_path, source_id, provider_id, remote_id).await {
            Ok(info) => {
                if let Some(s) = movie.sources.iter_mut().find(|s| s.id == source_id) {
                    s.duration_s = info.duration_seconds;
                    s.video_codec = info.video_codec;
                    s.audio_codec = info.audio_codec;
                }
            }
            Err(err) => tracing::warn!(movie_id = id, source_id, error = %err, "no se pudo comprobar la información técnica"),
        }
    }

    Ok(Json(movie.into()))
}

/// Lee la clave de TMDB del keychain (si no hay ninguna configurada, falla rápido y sin
/// red — igual que `commands::spawn_tmdb_matching`), pide tráiler+clasificación en una
/// sola llamada, y los guarda juntos.
async fn fetch_and_store_movie_extras(db_path: PathBuf, movie_id: i64, tmdb_id: i64) -> Result<TmdbExtras, String> {
    let api_key = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).map_err(|err| err.to_string())?;
    let extras = TmdbClient::new(api_key).movie_extras(tmdb_id).await.map_err(|err| err.to_string())?;
    let stored = extras.clone();
    with_db(db_path, move |conn| queries::set_movie_extras(conn, movie_id, stored.trailer_key.as_deref(), stored.age_rating.as_deref()))
        .await
        .map_err(|err| err.to_string())?;
    Ok(extras)
}

/// Igual que `fetch_and_store_movie_extras`, pero para series (`/tv/{id}`).
async fn fetch_and_store_series_extras(db_path: PathBuf, series_id: i64, tmdb_id: i64) -> Result<TmdbExtras, String> {
    let api_key = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).map_err(|err| err.to_string())?;
    let extras = TmdbClient::new(api_key).tv_extras(tmdb_id).await.map_err(|err| err.to_string())?;
    let stored = extras.clone();
    with_db(db_path, move |conn| queries::set_series_extras(conn, series_id, stored.trailer_key.as_deref(), stored.age_rating.as_deref()))
        .await
        .map_err(|err| err.to_string())?;
    Ok(extras)
}

/// Duración/códecs reales de una fuente, sin abrir su stream (`Source::vod_info`) — el
/// mismo `build_source_for_provider` que ya usa `refresh_series_detail` para reconstruir
/// la fuente con las credenciales del proveedor desde el keychain.
async fn fetch_and_store_vod_info(db_path: PathBuf, movie_source_id: i64, provider_id: i64, remote_id: String) -> Result<VodInfo, String> {
    let provider_row = with_db(db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await
        .map_err(|err| err.to_string())?
        .ok_or("proveedor no encontrado")?;
    let source = crate::commands::build_source_for_provider(provider_row).await?;
    let info = source.vod_info(&remote_id).await.map_err(|err| err.to_string())?;
    let stored = info.clone();
    with_db(db_path, move |conn| {
        queries::set_movie_source_vod_info(conn, movie_source_id, stored.duration_seconds, stored.video_codec.as_deref(), stored.audio_codec.as_deref())
    })
    .await
    .map_err(|err| err.to_string())?;
    Ok(info)
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct GenreSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
}

impl From<reads::GenreNameRow> for GenreSummary {
    fn from(row: reads::GenreNameRow) -> Self {
        Self { id: row.id, name: row.name }
    }
}

#[derive(Deserialize)]
struct GenresQuery {
    /// "movie" o "tv" — los catálogos de género de TMDB no son exactamente el mismo
    /// (p. ej. "Acción y aventura" solo existe en el de series).
    kind: String,
}

async fn get_genres(State(state): State<CatalogState>, Query(query): Query<GenresQuery>) -> Result<Json<Vec<GenreSummary>>, ApiError> {
    let kind = if query.kind == "tv" { "tv" } else { "movie" };
    let rows = with_db(state.db_path, move |conn| reads::list_genre_names(conn, kind)).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct SeriesQuery {
    profile_id: i64,
    genre: Option<i64>,
    year: Option<i32>,
    sort: Option<String>,
    after: Option<i64>,
    after_key: Option<String>,
    limit: Option<i64>,
}

async fn get_series_list(State(state): State<CatalogState>, Query(query): Query<SeriesQuery>) -> Result<Json<SeriesPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let sort = parse_series_sort(query.sort.as_deref());
    let cursor = parse_page_cursor(series_sort_cursor_key(sort), query.after, query.after_key.as_deref());
    let after_key = query.after_key.clone();
    let items = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::list_series_page(conn, query.genre, query.year, sort, cursor, after_key.as_deref(), limit, include_adult)
    })
    .await?;
    let (next_cursor, next_cursor_key) = if items.len() as i64 == limit {
        match items.last() {
            Some(last) => {
                let (cursor, key) = reads::series_page_cursor(sort, last);
                (Some(cursor.id), key)
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    Ok(Json(SeriesPage { items: items.into_iter().map(Into::into).collect(), next_cursor, next_cursor_key }))
}

/// Si el detalle de la serie no se ha pedido nunca o hace más de 7 días (ver
/// `docs/DATA-MODEL.md`, `series_sources.detail_synced_at`), llama a `series_detail()`
/// del proveedor antes de responder. Por eso esta ruta puede tardar más que las demás.
#[derive(Deserialize)]
struct SeriesDetailQuery {
    profile_id: i64,
}

async fn get_series_detail(
    State(state): State<CatalogState>,
    AxumPath(id): AxumPath<i64>,
    Query(query): Query<SeriesDetailQuery>,
) -> Result<Json<SeriesDetailResponse>, ApiError> {
    const SEVEN_DAYS_SECONDS: i64 = 7 * 24 * 3600;

    let db_path = state.db_path.clone();
    let stale_source = with_db(db_path.clone(), move |conn| reads::oldest_series_source_needing_detail(conn, id, SEVEN_DAYS_SECONDS)).await?;

    if let Some((series_source_id, provider_id)) = stale_source {
        if let Err(err) = refresh_series_detail(db_path.clone(), series_source_id, provider_id).await {
            // No es fatal: se devuelve lo que ya hay en caché aunque el refresco falle
            // (proveedor caído, credenciales revocadas, etc.).
            tracing::warn!(series_id = id, error = %err, "no se pudo refrescar el detalle de la serie");
        }
    }

    let series = with_db(db_path.clone(), move |conn| reads::get_series(conn, id)).await?;
    let mut series = series.ok_or(ApiError::NotFound)?;

    if series.trailer_checked_at.is_none() {
        if let Some(tmdb_id) = series.tmdb_id {
            match fetch_and_store_series_extras(db_path.clone(), id, tmdb_id).await {
                Ok(extras) => {
                    series.youtube_trailer_key = extras.trailer_key;
                    series.age_rating = extras.age_rating;
                }
                Err(err) => tracing::warn!(series_id = id, error = %err, "no se pudo comprobar el tráiler/clasificación"),
            }
        }
    }

    // Checkmark de "visto" por episodio (F "premium") — nunca rompe la ficha si falla:
    // sin marcas es como estaba la ficha antes de que existiera esta función.
    let profile_id = query.profile_id;
    let watched = with_db(db_path, move |conn| reads::list_watched_episode_ids(conn, profile_id, id)).await.unwrap_or_default();

    let mut episodes: Vec<EpisodeSummary> = series.episodes.into_iter().map(Into::into).collect();
    for episode in &mut episodes {
        episode.watched = watched.contains(&episode.id);
    }

    Ok(Json(SeriesDetailResponse {
        id: series.id,
        clean_title: series.clean_title,
        year: series.year,
        overview: series.overview,
        poster_path: series.poster_path,
        backdrop_path: series.backdrop_path,
        rating: series.rating,
        genres: series.genres,
        tmdb_state: series.tmdb_state,
        trailer_key: series.youtube_trailer_key,
        age_rating: series.age_rating,
        episodes,
    }))
}

async fn refresh_series_detail(db_path: PathBuf, series_source_id: i64, provider_id: i64) -> Result<(), String> {
    let (remote_id, provider_row) = with_db(db_path.clone(), move |conn| {
        let remote_id: String = conn.query_row("SELECT remote_id FROM series_sources WHERE id = ?1", rusqlite::params![series_source_id], |r| r.get(0))?;
        let provider_row = queries::get_provider(conn, provider_id)?;
        Ok((remote_id, provider_row))
    })
    .await
    .map_err(|err| err.to_string())?;

    let provider_row = provider_row.ok_or("proveedor no encontrado")?;
    let source = crate::commands::build_source_for_provider(provider_row).await?;
    let detail = source.series_detail(&remote_id).await.map_err(|err| err.to_string())?;

    with_db(db_path, move |conn| {
        reads::upsert_episodes(conn, series_source_id, &detail.episodes)?;
        reads::mark_series_source_detail_synced(conn, series_source_id)?;
        Ok(())
    })
    .await
    .map_err(|err| err.to_string())
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn get_search(State(state): State<CatalogState>, Query(query): Query<SearchQuery>) -> Result<Json<SearchResults>, ApiError> {
    let q = query.q.unwrap_or_default();
    if q.trim().is_empty() {
        return Ok(Json(SearchResults { channels: vec![], movies: vec![], series: vec![] }));
    }
    let hits = with_db(state.db_path, move |conn| reads::search(conn, &q, 20)).await?;

    let mut channels = Vec::new();
    let mut movies = Vec::new();
    let mut series = Vec::new();
    for hit in hits {
        let item = SearchHitResponse { item_kind: hit.item_kind.clone(), item_id: hit.item_id, title: hit.title, poster_path: hit.poster_path };
        match hit.item_kind.as_str() {
            "channel" => channels.push(item),
            "movie" => movies.push(item),
            "series" => series.push(item),
            _ => {}
        }
    }
    Ok(Json(SearchResults { channels, movies, series }))
}

// ─────────────── Guía EPG (F7) ───────────────

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct EpgProgrammeResponse {
    pub title: String,
    pub subtitle: Option<String>,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
}

impl From<reads::ProgrammeSummary> for EpgProgrammeResponse {
    fn from(row: reads::ProgrammeSummary) -> Self {
        Self { title: row.title, subtitle: row.subtitle, start_ts: row.start_ts, stop_ts: row.stop_ts }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct EpgNowNextResponse {
    #[ts(type = "number")]
    pub channel_id: i64,
    pub current: Option<EpgProgrammeResponse>,
    pub next: Option<EpgProgrammeResponse>,
}

#[derive(Deserialize)]
struct EpgNowQuery {
    channel_ids: String,
}

fn parse_channel_ids(raw: &str) -> Result<Vec<i64>, ApiError> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().parse::<i64>().map_err(|_| ApiError::BadRequest("channel_ids debe ser una lista de números separados por comas".to_string())))
        .collect()
}

/// Programa actual y siguiente de cada canal pedido. `channel_ids=1,2,3` (ids internos,
/// nunca el id de XMLTV — ver `db::reads`, sección de guía).
async fn get_epg_now(State(state): State<CatalogState>, Query(query): Query<EpgNowQuery>) -> Result<Json<Vec<EpgNowNextResponse>>, ApiError> {
    let channel_ids = parse_channel_ids(&query.channel_ids)?;
    let now = crate::db::now_epoch();
    let results = with_db(state.db_path, move |conn| reads::epg_now_and_next(conn, &channel_ids, now)).await?;
    Ok(Json(
        results
            .into_iter()
            .map(|r| EpgNowNextResponse { channel_id: r.channel_id, current: r.current.map(Into::into), next: r.next.map(Into::into) })
            .collect(),
    ))
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct EpgGridProgramme {
    #[ts(type = "number")]
    pub channel_id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
    pub category: Option<String>,
}

impl From<reads::GridProgramme> for EpgGridProgramme {
    fn from(row: reads::GridProgramme) -> Self {
        Self { channel_id: row.channel_id, title: row.title, subtitle: row.subtitle, start_ts: row.start_ts, stop_ts: row.stop_ts, category: row.category }
    }
}

#[derive(Deserialize)]
struct EpgGridQuery {
    channel_ids: String,
    from: i64,
    to: i64,
}

/// Vista de guía en rejilla: todos los programas de los canales pedidos que se solapan
/// con `[from, to)` (epoch, segundos).
async fn get_epg_grid(State(state): State<CatalogState>, Query(query): Query<EpgGridQuery>) -> Result<Json<Vec<EpgGridProgramme>>, ApiError> {
    if query.to <= query.from {
        return Err(ApiError::BadRequest("to debe ser mayor que from".to_string()));
    }
    let channel_ids = parse_channel_ids(&query.channel_ids)?;
    let (from, to) = (query.from, query.to);
    let results = with_db(state.db_path, move |conn| reads::epg_grid(conn, &channel_ids, from, to)).await?;
    Ok(Json(results.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct PlaybackProgressQuery {
    profile_id: i64,
    item_kind: String,
    #[serde(rename = "item_id")]
    item_id: i64,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PlaybackProgressResponse {
    #[ts(type = "number")]
    pub position_s: i64,
    #[ts(type = "number | null")]
    pub duration_s: Option<i64>,
    pub finished: bool,
}

/// El reproductor la consulta al abrir un ítem, para ofrecer "reanudar desde X" (F6).
/// Sin progreso guardado, responde 204 en vez de un objeto vacío: es más claro para el
/// cliente que comprobar un campo nulo.
async fn get_playback_progress(State(state): State<CatalogState>, Query(query): Query<PlaybackProgressQuery>) -> Result<Response, ApiError> {
    let progress =
        with_db(state.db_path, move |conn| reads::get_playback_progress(conn, query.profile_id, &query.item_kind, query.item_id)).await?;

    Ok(match progress {
        Some(p) => Json(PlaybackProgressResponse { position_s: p.position_s, duration_s: p.duration_s, finished: p.finished }).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn post_playback_progress(State(state): State<CatalogState>, Json(input): Json<PlaybackProgressInput>) -> Result<StatusCode, ApiError> {
    if !matches!(input.item_kind.as_str(), "movie" | "episode" | "channel") {
        return Err(ApiError::BadRequest("item_kind debe ser movie, episode o channel".to_string()));
    }
    let item_kind = input.item_kind.clone();
    with_db(state.db_path.clone(), move |conn| {
        reads::upsert_playback_progress(conn, input.profile_id, &input.item_kind, input.item_id, input.position_s, input.duration_s)
    })
    .await?;
    tokio::spawn(crate::sync::engine::push_playback_progress(
        state.db_path,
        input.profile_id,
        item_kind,
        input.item_id,
        input.position_s,
        input.duration_s,
    ));
    Ok(StatusCode::NO_CONTENT)
}

async fn get_favorites(State(state): State<CatalogState>, Query(query): Query<ProfileQuery>) -> Result<Json<Vec<FavoriteItem>>, ApiError> {
    let items = with_db(state.db_path, move |conn| reads::list_favorites(conn, query.profile_id)).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn post_favorite(State(state): State<CatalogState>, Json(input): Json<FavoriteInput>) -> Result<StatusCode, ApiError> {
    let item_kind = input.item_kind.clone();
    with_db(state.db_path.clone(), move |conn| reads::add_favorite(conn, input.profile_id, &input.item_kind, input.item_id)).await?;
    tokio::spawn(crate::sync::engine::push_favorite_add(state.db_path, input.profile_id, item_kind, input.item_id));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DeleteFavoriteQuery {
    profile_id: i64,
    item_kind: String,
    item_id: i64,
}

async fn delete_favorite(State(state): State<CatalogState>, Query(query): Query<DeleteFavoriteQuery>) -> Result<StatusCode, ApiError> {
    // El item_key hace falta ANTES de borrar la fila local (server::api::delete_favorite
    // ya no podría resolverlo después) — ver sync::engine::push_favorite_remove_by_key.
    let item_key = with_db(state.db_path.clone(), {
        let item_kind = query.item_kind.clone();
        move |conn| reads::item_key_for(conn, &item_kind, query.item_id)
    })
    .await?;
    let item_kind = query.item_kind.clone();
    with_db(state.db_path.clone(), move |conn| reads::remove_favorite(conn, query.profile_id, &query.item_kind, query.item_id)).await?;
    if let Some(item_key) = item_key {
        tokio::spawn(crate::sync::engine::push_favorite_remove_by_key(state.db_path, query.profile_id, item_kind, item_key));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Quitar algo de "Continuar viendo" a mano (F "premium"). Mismo `PlaybackProgressQuery`
/// que ya usa `GET /api/playback/progress` — profile_id/item_kind/item_id es justo lo
/// que hace falta para borrar la fila.
async fn delete_playback_progress(State(state): State<CatalogState>, Query(query): Query<PlaybackProgressQuery>) -> Result<StatusCode, ApiError> {
    let item_key = with_db(state.db_path.clone(), {
        let item_kind = query.item_kind.clone();
        move |conn| reads::item_key_for(conn, &item_kind, query.item_id)
    })
    .await?;
    let item_kind = query.item_kind.clone();
    with_db(state.db_path.clone(), move |conn| reads::remove_playback_progress(conn, query.profile_id, &query.item_kind, query.item_id)).await?;
    if let Some(item_key) = item_key {
        tokio::spawn(crate::sync::engine::push_playback_remove_by_key(state.db_path, query.profile_id, item_kind, item_key));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn catalog_router(db_path: PathBuf) -> Router {
    Router::new()
        .route("/api/home", get(get_home))
        .route("/api/live/categories", get(get_live_categories))
        .route("/api/live/channels", get(get_live_channels))
        .route("/api/movies", get(get_movies))
        .route("/api/movies/:id", get(get_movie_detail))
        .route("/api/series", get(get_series_list))
        .route("/api/series/:id", get(get_series_detail))
        .route("/api/genres", get(get_genres))
        .route("/api/search", get(get_search))
        .route("/api/epg/now", get(get_epg_now))
        .route("/api/epg/grid", get(get_epg_grid))
        .route("/api/playback/progress", get(get_playback_progress).post(post_playback_progress).delete(delete_playback_progress))
        .route("/api/favorites", get(get_favorites).post(post_favorite).delete(delete_favorite))
        .with_state(CatalogState { db_path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{self as write_queries, ProviderKind, TitleFields};
    use crate::db::schema::open_connection_at;
    use crate::sources::model::RawMovie;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;

    /// Perfil que usan todos los tests de este módulo — una base de datos de prueba
    /// nueva siempre le da el id 1 (primera fila de una tabla `INTEGER PRIMARY KEY`).
    const TEST_PROFILE_ID: i64 = 1;

    fn movie_list_item(added_at: Option<i64>) -> reads::MovieListItem {
        reads::MovieListItem { id: 1, clean_title: "T".into(), year: None, poster_path: None, rating: None, tmdb_state: "matched".into(), added_at }
    }

    #[test]
    fn movie_summary_is_new_within_the_last_seven_days() {
        let recent = MovieSummary::from(movie_list_item(Some(crate::db::now_epoch() - 3600)));
        assert!(recent.is_new);

        let old = MovieSummary::from(movie_list_item(Some(crate::db::now_epoch() - 30 * 24 * 3600)));
        assert!(!old.is_new);

        let unknown = MovieSummary::from(movie_list_item(None));
        assert!(!unknown.is_new);
    }

    fn seeded_db() -> PathBuf {
        let path = std::env::temp_dir().join(format!("vibe-iptv-api-test-{}-{}.sqlite3", std::process::id(), rand_suffix()));
        let conn = open_connection_at(&path).unwrap();
        let profile_id = crate::db::reads::ensure_default_profile(&conn).unwrap();
        assert_eq!(profile_id, TEST_PROFILE_ID);
        let provider_id = write_queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let raw = RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "Peli de Prueba".into(),
            container_extension: None,
            logo_url: None,
            added_at: Some(1_700_000_000),
            rating: Some(7.0),
            tmdb_id: None,
        };
        let fields = TitleFields { clean_title: "Peli de Prueba", year: Some(2021), match_key: "peli-de-prueba-2021", quality: None, language: None };
        write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
        write_queries::rebuild_search_index(&conn).unwrap();
        path
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn get_movies_returns_seeded_movie() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri(format!("/api/movies?profile_id={TEST_PROFILE_ID}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["items"][0]["clean_title"], "Peli de Prueba");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_movie_detail_404_when_missing() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/movies/9999").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_file(&db_path);
    }

    /// La película sembrada no tiene `tmdb_id` (estado `pending`): confirma que la ficha
    /// responde sin intentar comprobar el tráiler (que fallaría sin clave de TMDB real
    /// configurada) y que `trailer_key` sale `null` en vez de romper la respuesta.
    #[tokio::test]
    async fn get_movie_detail_has_no_trailer_key_when_never_matched_with_tmdb() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/movies/1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["trailer_key"], serde_json::Value::Null);

        let _ = std::fs::remove_file(&db_path);
    }

    /// Si el tráiler ya se comprobó antes (`trailer_checked_at` con fecha), la ficha lo
    /// devuelve directamente de caché sin intentar preguntarle a TMDB otra vez.
    #[tokio::test]
    async fn get_movie_detail_returns_cached_trailer_key_without_rechecking() {
        let db_path = seeded_db();
        {
            let conn = open_connection_at(&db_path).unwrap();
            conn.execute("UPDATE movies SET tmdb_state = 'matched', tmdb_id = 603 WHERE id = 1", []).unwrap();
            write_queries::set_movie_extras(&conn, 1, Some("ya-en-cache"), Some("16")).unwrap();
        }
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/movies/1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["trailer_key"], "ya-en-cache");
        assert_eq!(json["age_rating"], "16");

        let _ = std::fs::remove_file(&db_path);
    }

    /// La película sembrada tiene una fuente de un proveedor sin credenciales guardadas
    /// (`seeded_db` no las pone) — `Source::vod_info` no se puede ni intentar sin ellas.
    /// La ficha tiene que responder igual, con `duration_s`/`video_codec` a `null`, no
    /// romperse por esto.
    #[tokio::test]
    async fn get_movie_detail_still_responds_when_the_provider_has_no_stored_credentials() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/movies/1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["sources"][0]["duration_s"], serde_json::Value::Null);

        let _ = std::fs::remove_file(&db_path);
    }

    /// Si la información técnica ya se comprobó antes (`vod_info_checked_at` con fecha),
    /// la ficha la devuelve de caché sin intentar reconstruir ninguna fuente.
    #[tokio::test]
    async fn get_movie_detail_returns_cached_vod_info_without_rechecking() {
        let db_path = seeded_db();
        {
            let conn = open_connection_at(&db_path).unwrap();
            let source_id: i64 = conn.query_row("SELECT id FROM movie_sources WHERE movie_id = 1", [], |r| r.get(0)).unwrap();
            write_queries::set_movie_source_vod_info(&conn, source_id, Some(7200), Some("h264"), Some("aac")).unwrap();
        }
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/movies/1").body(Body::empty()).unwrap()).await.unwrap();
        let json = body_json(response).await;
        assert_eq!(json["sources"][0]["duration_s"], 7200);
        assert_eq!(json["sources"][0]["video_codec"], "h264");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_search_finds_seeded_movie() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri("/api/search?q=Prueba").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["movies"][0]["title"], "Peli de Prueba");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_home_recommended_is_empty_without_any_favorite_or_watched_movie() {
        // Sin ninguna semilla (favorito, o visto pasado el 70%), no hay nada que
        // preguntarle a TMDB — la fila sale vacía, no un error ni un `null`.
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router.oneshot(Request::builder().uri(format!("/api/home?profile_id={TEST_PROFILE_ID}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["recommended"], serde_json::json!([]));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn playback_progress_and_home_continue_watching_roundtrip() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let body = serde_json::json!({"profile_id": TEST_PROFILE_ID, "item_kind": "movie", "item_id": 1, "position_s": 30, "duration_s": 100});
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/playback/progress")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response =
            router.oneshot(Request::builder().uri(format!("/api/home?profile_id={TEST_PROFILE_ID}")).body(Body::empty()).unwrap()).await.unwrap();
        let json = body_json(response).await;
        assert_eq!(json["continue_watching"][0]["item_id"], 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_playback_progress_returns_204_without_saved_position_and_data_after() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/playback/progress?profile_id={TEST_PROFILE_ID}&item_kind=movie&item_id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let body = serde_json::json!({"profile_id": TEST_PROFILE_ID, "item_kind": "movie", "item_id": 1, "position_s": 42, "duration_s": 100});
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/playback/progress")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/playback/progress?profile_id={TEST_PROFILE_ID}&item_kind=movie&item_id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["position_s"], 42);

        let _ = std::fs::remove_file(&db_path);
    }

    /// "Quitar de continuar viendo" (F "premium"): borra el progreso entero, no solo lo
    /// oculta — reabrir el mismo título después debe empezar desde el principio.
    #[tokio::test]
    async fn delete_playback_progress_removes_it_entirely() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let body = serde_json::json!({"profile_id": TEST_PROFILE_ID, "item_kind": "movie", "item_id": 1, "position_s": 42, "duration_s": 100});
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/playback/progress")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/playback/progress?profile_id={TEST_PROFILE_ID}&item_kind=movie&item_id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let get_response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/playback/progress?profile_id={TEST_PROFILE_ID}&item_kind=movie&item_id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::NO_CONTENT);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn favorites_post_get_delete_roundtrip() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let body = serde_json::json!({"profile_id": TEST_PROFILE_ID, "item_kind": "movie", "item_id": 1});
        router
            .clone()
            .oneshot(Request::builder().method("POST").uri("/api/favorites").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();

        let response = router
            .clone()
            .oneshot(Request::builder().uri(format!("/api/favorites?profile_id={TEST_PROFILE_ID}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json = body_json(response).await;
        assert_eq!(json[0]["item_id"], 1);

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/favorites?profile_id={TEST_PROFILE_ID}&item_kind=movie&item_id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn movies_pagination_respects_limit_and_cursor() {
        let db_path = seeded_db();
        {
            let conn = open_connection_at(&db_path).unwrap();
            let provider_id = write_queries::insert_provider(&conn, ProviderKind::Xtream, "P2", "http://y", None, 1, None).unwrap();
            for i in 0..3 {
                let raw = RawMovie {
                    remote_id: format!("extra-{i}"),
                    category_remote_id: None,
                    raw_title: format!("Extra {i}"),
                    container_extension: None,
                    logo_url: None,
                    added_at: Some(1_700_000_100 + i),
                    rating: None,
                    tmdb_id: None,
                };
                let match_key = format!("extra-{i}-0000");
                let fields = TitleFields { clean_title: &format!("Extra {i}"), year: None, match_key: &match_key, quality: None, language: None };
                write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
            }
        }

        let router = catalog_router(db_path.clone());
        let response = router
            .oneshot(Request::builder().uri(format!("/api/movies?profile_id={TEST_PROFILE_ID}&limit=2")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json = body_json(response).await;
        assert_eq!(json["items"].as_array().unwrap().len(), 2);
        assert!(json["next_cursor"].is_number());

        let _ = std::fs::remove_file(&db_path);
    }

    /// Cubre justo el fallo que se encontró probando en vivo: `next_cursor_key` tiene
    /// que viajar de verdad en la respuesta HTTP para un orden que no sea "recientes" —
    /// si se queda `null`, la página siguiente no tiene con qué reconstruir el cursor
    /// compuesto y el filtro deja de aplicarse en silencio.
    #[tokio::test]
    async fn movies_pagination_by_rating_round_trips_the_cursor_key_over_http() {
        let db_path = seeded_db(); // "Peli de Prueba", sin rating (None).
        {
            let conn = open_connection_at(&db_path).unwrap();
            let provider_id = write_queries::insert_provider(&conn, ProviderKind::Xtream, "P2", "http://y", None, 1, None).unwrap();
            for (title, rating) in [("Alta", 9.0), ("Media", 5.0)] {
                let raw = RawMovie {
                    remote_id: title.to_string(),
                    category_remote_id: None,
                    raw_title: title.to_string(),
                    container_extension: None,
                    logo_url: None,
                    added_at: None,
                    rating: None,
                    tmdb_id: None,
                };
                let match_key = format!("{}-0000", title.to_lowercase());
                let fields = TitleFields { clean_title: title, year: None, match_key: &match_key, quality: None, language: None };
                write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
                // `upsert_movie` no escribe `rating` (solo lo rellena el emparejamiento
                // con TMDB) — se fija a mano para no depender de esa otra pieza aquí.
                conn.execute("UPDATE movies SET rating = ?1 WHERE match_key = ?2", rusqlite::params![rating, match_key]).unwrap();
            }
        }

        let router = catalog_router(db_path.clone());
        let first = router
            .clone()
            .oneshot(Request::builder().uri(format!("/api/movies?profile_id={TEST_PROFILE_ID}&sort=rating&limit=1")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let first_json = body_json(first).await;
        assert_eq!(first_json["items"][0]["clean_title"], "Alta");
        let next_cursor = first_json["next_cursor"].as_i64().unwrap();
        let next_cursor_key = first_json["next_cursor_key"].as_str().unwrap();
        assert_eq!(next_cursor_key.parse::<f64>().unwrap(), 9.0); // el formato exacto (Rust omite ".0") no es lo que importa aquí.

        let second = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/movies?profile_id={TEST_PROFILE_ID}&sort=rating&limit=1&after={next_cursor}&after_key={next_cursor_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let second_json = body_json(second).await;
        // Sin el cursor arreglado, esta segunda página repetiría "Alta" en vez de
        // avanzar a la siguiente por nota.
        assert_eq!(second_json["items"][0]["clean_title"], "Media");

        let _ = std::fs::remove_file(&db_path);
    }

    fn seed_channel_with_epg(db_path: &std::path::Path, epg_channel_id: &str) -> i64 {
        use crate::sources::model::RawChannel;
        let conn = open_connection_at(db_path).unwrap();
        let provider_id = write_queries::insert_provider(&conn, ProviderKind::Xtream, "P2", "http://y", None, 1, None).unwrap();
        let channel = RawChannel {
            remote_id: "101".into(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: None,
            epg_channel_id: Some(epg_channel_id.into()),
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        write_queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();
        conn.query_row("SELECT id FROM channels WHERE remote_id = '101'", [], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn get_epg_now_returns_current_and_next_programme() {
        let db_path = seeded_db();
        let channel_id = seed_channel_with_epg(&db_path, "canaluno.es");
        {
            // El handler usa la hora real (crate::db::now_epoch()), así que el
            // programa "actual" tiene que estar centrado en el instante real del test,
            // no en un epoch pequeño arbitrario.
            let now = crate::db::now_epoch();
            let conn = open_connection_at(&db_path).unwrap();
            conn.execute(
                "INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title) VALUES ('canaluno.es', ?1, ?2, 'Ahora')",
                rusqlite::params![now - 500, now + 500],
            )
            .unwrap();
        }

        let router = catalog_router(db_path.clone());
        let response = router
            .oneshot(Request::builder().uri(format!("/api/epg/now?channel_ids={channel_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json[0]["current"]["title"], "Ahora");
        assert!(json[0]["next"].is_null());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_epg_now_rejects_non_numeric_channel_ids() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());
        let response =
            router.oneshot(Request::builder().uri("/api/epg/now?channel_ids=abc").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_epg_grid_returns_programmes_in_window() {
        let db_path = seeded_db();
        let channel_id = seed_channel_with_epg(&db_path, "canaluno.es");
        {
            let conn = open_connection_at(&db_path).unwrap();
            conn.execute(
                "INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title) VALUES ('canaluno.es', 1000, 2000, 'Programa')",
                [],
            )
            .unwrap();
        }

        let router = catalog_router(db_path.clone());
        let response = router
            .oneshot(Request::builder().uri(format!("/api/epg/grid?channel_ids={channel_id}&from=0&to=5000")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json[0]["title"], "Programa");
        assert_eq!(json[0]["channel_id"], channel_id);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_epg_grid_rejects_backwards_window() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());
        let response = router
            .oneshot(Request::builder().uri("/api/epg/grid?channel_ids=1&from=100&to=50").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&db_path);
    }

}
