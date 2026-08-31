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

use crate::db::reads::{self, MovieSort};
use crate::db::{queries, schema, DbError};

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
}

impl From<reads::MovieListItem> for MovieSummary {
    fn from(row: reads::MovieListItem) -> Self {
        Self { id: row.id, clean_title: row.clean_title, year: row.year, poster_path: row.poster_path, rating: row.rating, tmdb_state: row.tmdb_state }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct MoviesPage {
    pub items: Vec<MovieSummary>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
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
}

impl From<reads::MovieSourceItem> for MovieSourceSummary {
    fn from(row: reads::MovieSourceItem) -> Self {
        Self { id: row.id, provider_id: row.provider_id, remote_id: row.remote_id, quality: row.quality, language: row.language, container: row.container }
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
}

impl From<reads::EpisodeItem> for EpisodeSummary {
    fn from(row: reads::EpisodeItem) -> Self {
        Self { id: row.id, series_source_id: row.series_source_id, season: row.season, number: row.number, title: row.title, overview: row.overview, still_path: row.still_path, duration_s: row.duration_s }
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
    pub episodes: Vec<EpisodeSummary>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SearchHitResponse {
    pub item_kind: String,
    #[ts(type = "number")]
    pub item_id: i64,
    pub title: String,
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
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct HomeResponse {
    pub continue_watching: Vec<ContinueWatchingItem>,
    pub recent_movies: Vec<MovieSummary>,
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PlaybackProgressInput {
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
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct FavoriteInput {
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

async fn get_home(State(state): State<CatalogState>) -> Result<Json<HomeResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let (continue_watching, recent_movies) = with_db(db_path, |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        let continue_watching = reads::list_continue_watching(conn, profile_id, 20)?;
        let recent_movies = reads::list_recent_movies(conn, 20)?;
        Ok((continue_watching, recent_movies))
    })
    .await?;

    Ok(Json(HomeResponse {
        continue_watching: continue_watching
            .into_iter()
            .map(|p| ContinueWatchingItem { item_kind: p.item_kind, item_id: p.item_id, position_s: p.position_s, duration_s: p.duration_s })
            .collect(),
        recent_movies: recent_movies.into_iter().map(Into::into).collect(),
    }))
}

async fn get_live_categories(State(state): State<CatalogState>) -> Result<Json<Vec<CategorySummary>>, ApiError> {
    let items = with_db(state.db_path, |conn| reads::list_categories(conn, "live")).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct ChannelsQuery {
    category_id: Option<i64>,
    q: Option<String>,
    after: Option<i64>,
    limit: Option<i64>,
}

async fn get_live_channels(State(state): State<CatalogState>, Query(query): Query<ChannelsQuery>) -> Result<Json<ChannelsPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let items = with_db(state.db_path, move |conn| {
        reads::list_channels_page(conn, query.category_id, query.q.as_deref(), query.after, limit)
    })
    .await?;
    let next_cursor = if items.len() as i64 == limit { items.last().map(|c| c.id) } else { None };
    Ok(Json(ChannelsPage { items: items.into_iter().map(Into::into).collect(), next_cursor }))
}

#[derive(Deserialize)]
struct MoviesQuery {
    genre: Option<i64>,
    year: Option<i32>,
    quality: Option<String>,
    sort: Option<String>,
    after: Option<i64>,
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

async fn get_movies(State(state): State<CatalogState>, Query(query): Query<MoviesQuery>) -> Result<Json<MoviesPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let sort = parse_movie_sort(query.sort.as_deref());
    let items = with_db(state.db_path, move |conn| {
        reads::list_movies_page(conn, query.genre, query.year, query.quality.as_deref(), sort, query.after, limit)
    })
    .await?;
    let next_cursor = if items.len() as i64 == limit { items.last().map(|m| m.id) } else { None };
    Ok(Json(MoviesPage { items: items.into_iter().map(Into::into).collect(), next_cursor }))
}

async fn get_movie_detail(State(state): State<CatalogState>, AxumPath(id): AxumPath<i64>) -> Result<Json<MovieDetailResponse>, ApiError> {
    let movie = with_db(state.db_path, move |conn| reads::get_movie(conn, id)).await?;
    movie.map(|m| Json(m.into())).ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
struct SeriesQuery {
    genre: Option<i64>,
    year: Option<i32>,
    after: Option<i64>,
    limit: Option<i64>,
}

async fn get_series_list(State(state): State<CatalogState>, Query(query): Query<SeriesQuery>) -> Result<Json<SeriesPage>, ApiError> {
    let limit = clamp_limit(query.limit);
    let items = with_db(state.db_path, move |conn| reads::list_series_page(conn, query.genre, query.year, query.after, limit)).await?;
    let next_cursor = if items.len() as i64 == limit { items.last().map(|s| s.id) } else { None };
    Ok(Json(SeriesPage { items: items.into_iter().map(Into::into).collect(), next_cursor }))
}

/// Si el detalle de la serie no se ha pedido nunca o hace más de 7 días (ver
/// `docs/DATA-MODEL.md`, `series_sources.detail_synced_at`), llama a `series_detail()`
/// del proveedor antes de responder. Por eso esta ruta puede tardar más que las demás.
async fn get_series_detail(State(state): State<CatalogState>, AxumPath(id): AxumPath<i64>) -> Result<Json<SeriesDetailResponse>, ApiError> {
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

    let series = with_db(db_path, move |conn| reads::get_series(conn, id)).await?;
    let series = series.ok_or(ApiError::NotFound)?;
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
        episodes: series.episodes.into_iter().map(Into::into).collect(),
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
        let item = SearchHitResponse { item_kind: hit.item_kind.clone(), item_id: hit.item_id, title: hit.title };
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
    let progress = with_db(state.db_path, move |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        reads::get_playback_progress(conn, profile_id, &query.item_kind, query.item_id)
    })
    .await?;

    Ok(match progress {
        Some(p) => Json(PlaybackProgressResponse { position_s: p.position_s, duration_s: p.duration_s, finished: p.finished }).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn post_playback_progress(State(state): State<CatalogState>, Json(input): Json<PlaybackProgressInput>) -> Result<StatusCode, ApiError> {
    if !matches!(input.item_kind.as_str(), "movie" | "episode" | "channel") {
        return Err(ApiError::BadRequest("item_kind debe ser movie, episode o channel".to_string()));
    }
    with_db(state.db_path, move |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        reads::upsert_playback_progress(conn, profile_id, &input.item_kind, input.item_id, input.position_s, input.duration_s)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_favorites(State(state): State<CatalogState>) -> Result<Json<Vec<FavoriteItem>>, ApiError> {
    let items = with_db(state.db_path, |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        reads::list_favorites(conn, profile_id)
    })
    .await?;
    Ok(Json(items.into_iter().map(|(item_kind, item_id)| FavoriteItem { item_kind, item_id }).collect()))
}

async fn post_favorite(State(state): State<CatalogState>, Json(input): Json<FavoriteInput>) -> Result<StatusCode, ApiError> {
    with_db(state.db_path, move |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        reads::add_favorite(conn, profile_id, &input.item_kind, input.item_id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DeleteFavoriteQuery {
    item_kind: String,
    item_id: i64,
}

async fn delete_favorite(State(state): State<CatalogState>, Query(query): Query<DeleteFavoriteQuery>) -> Result<StatusCode, ApiError> {
    with_db(state.db_path, move |conn| {
        let profile_id = reads::ensure_default_profile(conn)?;
        reads::remove_favorite(conn, profile_id, &query.item_kind, query.item_id)
    })
    .await?;
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
        .route("/api/search", get(get_search))
        .route("/api/epg/now", get(get_epg_now))
        .route("/api/epg/grid", get(get_epg_grid))
        .route("/api/playback/progress", get(get_playback_progress).post(post_playback_progress))
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

    fn seeded_db() -> PathBuf {
        let path = std::env::temp_dir().join(format!("vibe-iptv-api-test-{}-{}.sqlite3", std::process::id(), rand_suffix()));
        let conn = open_connection_at(&path).unwrap();
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

        let response = router.oneshot(Request::builder().uri("/api/movies").body(Body::empty()).unwrap()).await.unwrap();
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
    async fn playback_progress_and_home_continue_watching_roundtrip() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let body = serde_json::json!({"item_kind": "movie", "item_id": 1, "position_s": 30, "duration_s": 100});
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

        let response = router.oneshot(Request::builder().uri("/api/home").body(Body::empty()).unwrap()).await.unwrap();
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
            .oneshot(Request::builder().uri("/api/playback/progress?item_kind=movie&item_id=1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let body = serde_json::json!({"item_kind": "movie", "item_id": 1, "position_s": 42, "duration_s": 100});
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
            .oneshot(Request::builder().uri("/api/playback/progress?item_kind=movie&item_id=1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["position_s"], 42);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn favorites_post_get_delete_roundtrip() {
        let db_path = seeded_db();
        let router = catalog_router(db_path.clone());

        let body = serde_json::json!({"item_kind": "movie", "item_id": 1});
        router
            .clone()
            .oneshot(Request::builder().method("POST").uri("/api/favorites").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();

        let response = router.clone().oneshot(Request::builder().uri("/api/favorites").body(Body::empty()).unwrap()).await.unwrap();
        let json = body_json(response).await;
        assert_eq!(json[0]["item_id"], 1);

        let response = router
            .oneshot(Request::builder().method("DELETE").uri("/api/favorites?item_kind=movie&item_id=1").body(Body::empty()).unwrap())
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
        let response = router.oneshot(Request::builder().uri("/api/movies?limit=2").body(Body::empty()).unwrap()).await.unwrap();
        let json = body_json(response).await;
        assert_eq!(json["items"].as_array().unwrap().len(), 2);
        assert!(json["next_cursor"].is_number());

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
