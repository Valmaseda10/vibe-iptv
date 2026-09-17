//! Cliente TMDB: búsqueda con similitud + año, caché de aciertos y fallos, uso directo
//! de `tmdb_id` si el proveedor lo da, 5 peticiones concurrentes con reintento
//! exponencial ante 429. Ver "Emparejamiento con TMDB" en `docs/DATA-MODEL.md`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::db::{self, queries};
use crate::sources::MediaKind;

/// Clave de keychain bajo la que vive la clave de API de TMDB (regla R5: nunca en
/// SQLite). Vive aquí (no en `commands.rs`, donde se definió al principio) porque tanto
/// los comandos de Tauri como los handlers HTTP de `server::api` (F "premium": tráiler
/// bajo demanda al abrir una ficha) necesitan poder leerla.
pub const TMDB_API_KEY_CREDENTIAL_REF: &str = "tmdb-api-key";

const DEFAULT_BASE_URL: &str = "https://api.themoviedb.org/3";
const MAX_CONCURRENT: usize = 5;
const MAX_RETRIES: u32 = 4;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(200);
const SIMILARITY_THRESHOLD: f64 = 0.85;
const YEAR_MARGIN: i32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum TmdbError {
    #[error("error de red hacia TMDB: {0}")]
    Http(String),
    #[error("respuesta inesperada de TMDB: {0}")]
    InvalidResponse(String),
    #[error("límite de peticiones de TMDB agotado tras reintentar")]
    RateLimited,
    #[error("el semáforo de peticiones a TMDB se cerró inesperadamente")]
    SemaphoreClosed,
    #[error("error de base de datos: {0}")]
    Db(#[from] db::DbError),
}

pub type Result<T> = std::result::Result<T, TmdbError>;

/// Resultado aceptado de una búsqueda o de una consulta directa por `tmdb_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TmdbMatch {
    pub tmdb_id: i64,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    pub genre_ids: Vec<i64>,
    /// Solo se rellena vía `movie_details` (la búsqueda no lo trae). Ver docs/DECISIONS.md.
    pub runtime_min: Option<i32>,
}

/// Resultado crudo de `search/movie` o `search/tv`, antes de decidir si se acepta.
#[derive(Debug, Clone)]
pub struct TmdbSearchHit {
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub rating: Option<f64>,
    pub genre_ids: Vec<i64>,
}

impl From<TmdbSearchHit> for TmdbMatch {
    fn from(hit: TmdbSearchHit) -> Self {
        Self {
            tmdb_id: hit.id,
            overview: hit.overview,
            poster_path: hit.poster_path,
            backdrop_path: hit.backdrop_path,
            rating: hit.rating,
            genre_ids: hit.genre_ids,
            runtime_min: None,
        }
    }
}

pub struct TmdbClient {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl TmdbClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Para tests: apunta a un servidor local en vez de a la API real.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            client: reqwest::Client::new(),
            semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT)),
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str, params: &[(&str, &str)]) -> Result<T> {
        let _permit = self.semaphore.acquire().await.map_err(|_| TmdbError::SemaphoreClosed)?;

        let mut url = reqwest::Url::parse(&format!("{}{}", self.base_url, path))
            .map_err(|err| TmdbError::InvalidResponse(err.to_string()))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("api_key", &self.api_key);
            for (key, value) in params {
                query.append_pair(key, value);
            }
        }

        let mut delay = INITIAL_RETRY_DELAY;
        for attempt in 0..MAX_RETRIES {
            let response = self.client.get(url.clone()).send().await.map_err(|err| TmdbError::Http(err.to_string()))?;

            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                if attempt + 1 == MAX_RETRIES {
                    return Err(TmdbError::RateLimited);
                }
                tokio::time::sleep(delay).await;
                delay *= 2;
                continue;
            }
            if !response.status().is_success() {
                return Err(TmdbError::InvalidResponse(format!("HTTP {}", response.status())));
            }
            return response.json::<T>().await.map_err(|err| TmdbError::InvalidResponse(err.to_string()));
        }
        Err(TmdbError::RateLimited)
    }

    pub async fn search_movie(&self, query: &str, year: Option<i32>, language: &str) -> Result<Vec<TmdbSearchHit>> {
        let year_str = year.map(|y| y.to_string());
        let mut params: Vec<(&str, &str)> = vec![("query", query), ("language", language)];
        if let Some(y) = &year_str {
            params.push(("year", y));
        }
        let response: MovieSearchResponse = self.get_json("/search/movie", &params).await?;
        Ok(response.results.into_iter().map(MovieSearchHit::into_hit).collect())
    }

    pub async fn search_tv(&self, query: &str, year: Option<i32>, language: &str) -> Result<Vec<TmdbSearchHit>> {
        let year_str = year.map(|y| y.to_string());
        let mut params: Vec<(&str, &str)> = vec![("query", query), ("language", language)];
        if let Some(y) = &year_str {
            params.push(("first_air_date_year", y));
        }
        let response: TvSearchResponse = self.get_json("/search/tv", &params).await?;
        Ok(response.results.into_iter().map(TvSearchHit::into_hit).collect())
    }

    pub async fn movie_details(&self, tmdb_id: i64, language: &str) -> Result<TmdbMatch> {
        let response: MovieDetailsResponse =
            self.get_json(&format!("/movie/{tmdb_id}"), &[("language", language)]).await?;
        Ok(TmdbMatch {
            tmdb_id: response.id,
            overview: response.overview,
            poster_path: response.poster_path,
            backdrop_path: response.backdrop_path,
            rating: response.vote_average,
            genre_ids: response.genres.into_iter().map(|g| g.id).collect(),
            runtime_min: response.runtime,
        })
    }

    pub async fn tv_details(&self, tmdb_id: i64, language: &str) -> Result<TmdbMatch> {
        let response: TvDetailsResponse = self.get_json(&format!("/tv/{tmdb_id}"), &[("language", language)]).await?;
        Ok(TmdbMatch {
            tmdb_id: response.id,
            overview: response.overview,
            poster_path: response.poster_path,
            backdrop_path: response.backdrop_path,
            rating: response.vote_average,
            genre_ids: response.genres.into_iter().map(|g| g.id).collect(),
            runtime_min: None,
        })
    }

    /// Tráiler + clasificación por edad en una sola llamada (`append_to_response`, en
    /// vez de dos peticiones sueltas): ambos se comprueban juntos, bajo demanda, al abrir
    /// la ficha (`server::api`) — nunca durante el emparejamiento masivo. Con catálogos
    /// de hasta 100.000 títulos (regla R4) no tiene sentido gastar peticiones de más en
    /// contenido que puede que nadie abra nunca. Ninguno de los dos es un error si no
    /// hay dato: TMDB simplemente no siempre tiene tráiler o certificación para todo.
    pub async fn movie_extras(&self, tmdb_id: i64) -> Result<TmdbExtras> {
        let response: MovieExtrasResponse = self
            .get_json(&format!("/movie/{tmdb_id}"), &[("language", "es-ES"), ("append_to_response", "videos,release_dates,recommendations")])
            .await?;
        Ok(TmdbExtras {
            trailer_key: pick_trailer(&response.videos.results),
            age_rating: pick_certification(response.release_dates.results.iter().map(|c| {
                (c.iso_3166_1.as_str(), c.release_dates.iter().find_map(|r| non_empty(&r.certification)))
            })),
            similar_tmdb_ids: response.recommendations.results.into_iter().map(|r| r.id).collect(),
        })
    }

    pub async fn tv_extras(&self, tmdb_id: i64) -> Result<TmdbExtras> {
        let response: TvExtrasResponse = self
            .get_json(&format!("/tv/{tmdb_id}"), &[("language", "es-ES"), ("append_to_response", "videos,content_ratings,recommendations")])
            .await?;
        Ok(TmdbExtras {
            trailer_key: pick_trailer(&response.videos.results),
            age_rating: pick_certification(response.content_ratings.results.iter().map(|c| (c.iso_3166_1.as_str(), non_empty(&c.rating)))),
            similar_tmdb_ids: response.recommendations.results.into_iter().map(|r| r.id).collect(),
        })
    }

    /// Ids de TMDB recomendados a partir de una película — para "Recomendado para ti"
    /// (F "premium"): la lista se cruza después con el catálogo local (regla de oro
    /// del informe de I+D — recomendar algo que el proveedor no tiene es el error
    /// clásico), así que aquí solo hacen falta los ids, no toda la ficha de cada uno.
    pub async fn movie_recommendations(&self, tmdb_id: i64) -> Result<Vec<i64>> {
        let response: RecommendationsResponse =
            self.get_json(&format!("/movie/{tmdb_id}/recommendations"), &[("language", "es-ES")]).await?;
        Ok(response.results.into_iter().map(|r| r.id).collect())
    }

    /// Igual que `movie_recommendations`, para "Títulos similares" en la ficha de serie
    /// (F "premium" — hasta ahora esa fila solo existía en Inicio, no en la ficha).
    pub async fn tv_recommendations(&self, tmdb_id: i64) -> Result<Vec<i64>> {
        let response: RecommendationsResponse =
            self.get_json(&format!("/tv/{tmdb_id}/recommendations"), &[("language", "es-ES")]).await?;
        Ok(response.results.into_iter().map(|r| r.id).collect())
    }

    /// Los ~19 géneros de película y ~16 de serie de TMDB, casi estáticos — se piden una
    /// vez por sincronización con clave de TMDB (`commands::spawn_tmdb_matching`), nunca
    /// por título: es la misma llamada sirva para 10 películas o para 100.000.
    pub async fn movie_genres(&self, language: &str) -> Result<Vec<(i64, String)>> {
        let response: GenreListResponse = self.get_json("/genre/movie/list", &[("language", language)]).await?;
        Ok(response.genres.into_iter().map(|g| (g.id, g.name)).collect())
    }

    pub async fn tv_genres(&self, language: &str) -> Result<Vec<(i64, String)>> {
        let response: GenreListResponse = self.get_json("/genre/tv/list", &[("language", language)]).await?;
        Ok(response.genres.into_iter().map(|g| (g.id, g.name)).collect())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TmdbExtras {
    pub trailer_key: Option<String>,
    pub age_rating: Option<String>,
    pub similar_tmdb_ids: Vec<i64>,
}

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// España primero; si TMDB no tiene certificación para España, Estados Unidos (la
/// región con más cobertura de certificaciones en TMDB); si tampoco, la primera que
/// haya de cualquier país — mejor una clasificación aproximada que ninguna.
fn pick_certification<'a>(entries: impl Iterator<Item = (&'a str, Option<&'a str>)>) -> Option<String> {
    let mut us = None;
    let mut any = None;
    for (country, cert) in entries {
        let Some(cert) = cert else { continue };
        if country == "ES" {
            return Some(cert.to_string());
        }
        if country == "US" {
            us.get_or_insert(cert.to_string());
        }
        any.get_or_insert(cert.to_string());
    }
    us.or(any)
}

/// Prefiere un tráiler "oficial" en YouTube; si no hay ninguno marcado como oficial,
/// acepta el primer tráiler en YouTube que haya. Nunca un "Teaser"/"Clip"/"Featurette"
/// si hay un "Trailer" de verdad disponible.
fn pick_trailer(results: &[VideoWire]) -> Option<String> {
    let youtube_trailers: Vec<&VideoWire> = results.iter().filter(|v| v.site == "YouTube" && v.kind == "Trailer").collect();
    youtube_trailers
        .iter()
        .find(|v| v.official)
        .or_else(|| youtube_trailers.first())
        .map(|v| v.key.clone())
}

/// "Acepta el primer resultado si la similitud del título supera 0,85 y el año coincide
/// con un margen de un año." Si no hay año en alguno de los dos lados, no se comprueba.
pub fn choose_best_match<'a>(results: &'a [TmdbSearchHit], query: &str, year: Option<i32>) -> Option<&'a TmdbSearchHit> {
    let first = results.first()?;
    if similarity(&first.title, query) < SIMILARITY_THRESHOLD {
        return None;
    }
    if let (Some(q), Some(h)) = (year, first.year) {
        if (q - h).abs() > YEAR_MARGIN {
            return None;
        }
    }
    Some(first)
}

/// Similitud normalizada por distancia de Levenshtein, 0.0 a 1.0. Case-insensitive.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a = a.trim().to_lowercase();
    let b = b.trim().to_lowercase();
    if a.is_empty() || b.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let distance = levenshtein(&a, &b) as f64;
    let max_len = a.chars().count().max(b.chars().count()) as f64;
    1.0 - (distance / max_len)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum CachedOutcome {
    Matched(TmdbMatch),
    NotFound,
}

struct MatchOutcome {
    state: &'static str,
    data: Option<TmdbMatch>,
}

fn cache_key(kind: MediaKind, title: &str, year: Option<i32>) -> String {
    let kind_str = if kind == MediaKind::Movie { "movie" } else { "tv" };
    format!("{kind_str}:{}:{}", title.trim().to_lowercase(), year.map(|y| y.to_string()).unwrap_or_default())
}

async fn search_with_fallback(client: &TmdbClient, kind: MediaKind, title: &str, year: Option<i32>) -> Result<Vec<TmdbSearchHit>> {
    let hits = match kind {
        MediaKind::Movie => client.search_movie(title, year, "es-ES").await?,
        _ => client.search_tv(title, year, "es-ES").await?,
    };
    if !hits.is_empty() {
        return Ok(hits);
    }
    match kind {
        MediaKind::Movie => client.search_movie(title, year, "en-US").await,
        _ => client.search_tv(title, year, "en-US").await,
    }
}

async fn match_title(
    conn: &AsyncMutex<Connection>,
    client: &TmdbClient,
    kind: MediaKind,
    title: &str,
    year: Option<i32>,
    known_tmdb_id: Option<i64>,
) -> MatchOutcome {
    if let Some(id) = known_tmdb_id {
        // El proveedor ya lo dice: nos saltamos la búsqueda por título (docs/XTREAM-API.md).
        let details = match kind {
            MediaKind::Movie => client.movie_details(id, "es-ES").await,
            _ => client.tv_details(id, "es-ES").await,
        };
        return match details {
            Ok(m) => MatchOutcome { state: "matched", data: Some(m) },
            Err(_) => MatchOutcome { state: "error", data: None },
        };
    }

    let key = cache_key(kind, title, year);
    {
        let conn = conn.lock().await;
        if let Ok(Some(cached)) = queries::get_tmdb_cache(&conn, &key) {
            if let Ok(parsed) = serde_json::from_str::<CachedOutcome>(&cached) {
                return match parsed {
                    CachedOutcome::Matched(m) => MatchOutcome { state: "matched", data: Some(m) },
                    CachedOutcome::NotFound => MatchOutcome { state: "not_found", data: None },
                };
            }
        }
    }

    let hits = match search_with_fallback(client, kind, title, year).await {
        Ok(hits) => hits,
        Err(_) => return MatchOutcome { state: "error", data: None }, // transitorio: no se cachea
    };
    let accepted = choose_best_match(&hits, title, year).cloned().map(TmdbMatch::from);

    let cached_form = match &accepted {
        Some(m) => CachedOutcome::Matched(m.clone()),
        None => CachedOutcome::NotFound,
    };
    if let Ok(json) = serde_json::to_string(&cached_form) {
        let conn = conn.lock().await;
        let _ = queries::set_tmdb_cache(&conn, &key, &json);
    }

    match accepted {
        Some(m) => MatchOutcome { state: "matched", data: Some(m) },
        None => MatchOutcome { state: "not_found", data: None },
    }
}

async fn store_outcome(conn: &AsyncMutex<Connection>, kind: MediaKind, id: i64, outcome: &MatchOutcome) {
    let genres_json = outcome.data.as_ref().map(|m| serde_json::to_string(&m.genre_ids).unwrap_or_else(|_| "[]".to_string()));
    let db_outcome = queries::TmdbOutcome {
        state: outcome.state,
        tmdb_id: outcome.data.as_ref().map(|m| m.tmdb_id),
        overview: outcome.data.as_ref().and_then(|m| m.overview.as_deref()),
        poster_path: outcome.data.as_ref().and_then(|m| m.poster_path.as_deref()),
        backdrop_path: outcome.data.as_ref().and_then(|m| m.backdrop_path.as_deref()),
        rating: outcome.data.as_ref().and_then(|m| m.rating),
        genres_json: genres_json.as_deref(),
        runtime_min: outcome.data.as_ref().and_then(|m| m.runtime_min),
    };
    let conn = conn.lock().await;
    let _ = match kind {
        MediaKind::Movie => queries::update_movie_tmdb_result(&conn, id, &db_outcome),
        _ => queries::update_series_tmdb_result(&conn, id, &db_outcome),
    };
}

/// Pide los nombres de género de película y de serie a TMDB — sin tocar la base de
/// datos aquí: mantener una `&Connection` (que no es `Sync`) viva a través de un
/// `.await` dejaría el futuro entero sin poder mandarse entre hilos, y esto lo llama
/// `commands::spawn_tmdb_matching` dentro de una tarea de `tauri::async_runtime::spawn`
/// que sí lo exige. El guardado en sí (`queries::upsert_genre_names`, sin await) lo hace
/// quien llame, ya con la conexión en la mano.
pub async fn fetch_genre_names(client: &TmdbClient) -> (Vec<(i64, String)>, Vec<(i64, String)>) {
    let movie = client.movie_genres("es-ES").await.unwrap_or_default();
    let tv = client.tv_genres("es-ES").await.unwrap_or_default();
    (movie, tv)
}

pub struct MatchSummary {
    pub total: u32,
    pub matched: u32,
}

/// Recorre todas las películas y series en estado `pending`/`error` y las empareja con
/// TMDB. Corre después de `catalog::sync::run`, sin bloquear el catálogo ya sincronizado
/// (regla R4): se dispara desde `commands::spawn_sync`, no desde `sync::run`.
pub async fn run_matching_pass(
    conn: Connection,
    client: Arc<TmdbClient>,
    cancel: Arc<AtomicBool>,
    on_progress: Arc<dyn Fn(u32, u32) + Send + Sync>,
) -> Result<MatchSummary> {
    let conn = Arc::new(AsyncMutex::new(conn));

    let pending_movies = {
        let conn = conn.lock().await;
        queries::list_pending_movies(&conn)?
    };
    let pending_series = {
        let conn = conn.lock().await;
        queries::list_pending_series(&conn)?
    };

    let total = (pending_movies.len() + pending_series.len()) as u32;
    let done = Arc::new(AtomicU32::new(0));
    let matched = Arc::new(AtomicU32::new(0));
    let mut handles = Vec::with_capacity(total as usize);

    for movie in pending_movies {
        let conn = Arc::clone(&conn);
        let client = Arc::clone(&client);
        let cancel = Arc::clone(&cancel);
        let done = Arc::clone(&done);
        let matched = Arc::clone(&matched);
        let on_progress = Arc::clone(&on_progress);
        handles.push(tokio::spawn(async move {
            if !cancel.load(Ordering::SeqCst) {
                let outcome = match_title(&conn, &client, MediaKind::Movie, &movie.clean_title, movie.year, movie.tmdb_id).await;
                if outcome.state == "matched" {
                    matched.fetch_add(1, Ordering::SeqCst);
                }
                store_outcome(&conn, MediaKind::Movie, movie.id, &outcome).await;
            }
            let progressed = done.fetch_add(1, Ordering::SeqCst) + 1;
            on_progress(progressed, total);
        }));
    }

    for one_series in pending_series {
        let conn = Arc::clone(&conn);
        let client = Arc::clone(&client);
        let cancel = Arc::clone(&cancel);
        let done = Arc::clone(&done);
        let matched = Arc::clone(&matched);
        let on_progress = Arc::clone(&on_progress);
        handles.push(tokio::spawn(async move {
            if !cancel.load(Ordering::SeqCst) {
                let outcome =
                    match_title(&conn, &client, MediaKind::Series, &one_series.clean_title, one_series.year, one_series.tmdb_id).await;
                if outcome.state == "matched" {
                    matched.fetch_add(1, Ordering::SeqCst);
                }
                store_outcome(&conn, MediaKind::Series, one_series.id, &outcome).await;
            }
            let progressed = done.fetch_add(1, Ordering::SeqCst) + 1;
            on_progress(progressed, total);
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(MatchSummary { total, matched: matched.load(Ordering::SeqCst) })
}

// ─────────────── Formas del JSON de TMDB ───────────────

#[derive(Debug, Deserialize)]
struct MovieSearchResponse {
    #[serde(default)]
    results: Vec<MovieSearchHit>,
}

#[derive(Debug, Deserialize)]
struct MovieSearchHit {
    id: i64,
    title: String,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    genre_ids: Vec<i64>,
}

impl MovieSearchHit {
    fn into_hit(self) -> TmdbSearchHit {
        TmdbSearchHit {
            id: self.id,
            title: self.title,
            year: year_from_date(self.release_date.as_deref()),
            overview: self.overview,
            poster_path: self.poster_path,
            backdrop_path: self.backdrop_path,
            rating: self.vote_average,
            genre_ids: self.genre_ids,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TvSearchResponse {
    #[serde(default)]
    results: Vec<TvSearchHit>,
}

#[derive(Debug, Deserialize)]
struct TvSearchHit {
    id: i64,
    name: String,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    genre_ids: Vec<i64>,
}

impl TvSearchHit {
    fn into_hit(self) -> TmdbSearchHit {
        TmdbSearchHit {
            id: self.id,
            title: self.name,
            year: year_from_date(self.first_air_date.as_deref()),
            overview: self.overview,
            poster_path: self.poster_path,
            backdrop_path: self.backdrop_path,
            rating: self.vote_average,
            genre_ids: self.genre_ids,
        }
    }
}

fn year_from_date(date: Option<&str>) -> Option<i32> {
    date.filter(|d| d.len() >= 4).and_then(|d| d[0..4].parse().ok())
}

#[derive(Debug, Deserialize)]
struct GenreObj {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct MovieDetailsResponse {
    id: i64,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    genres: Vec<GenreObj>,
    #[serde(default)]
    runtime: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
struct VideosResponse {
    #[serde(default)]
    results: Vec<VideoWire>,
}

#[derive(Debug, Deserialize)]
struct VideoWire {
    key: String,
    site: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    official: bool,
}

#[derive(Debug, Deserialize)]
struct TvDetailsResponse {
    id: i64,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    genres: Vec<GenreObj>,
}

#[derive(Debug, Default, Deserialize)]
struct MovieExtrasResponse {
    #[serde(default)]
    videos: VideosResponse,
    #[serde(default)]
    release_dates: ReleaseDatesResponse,
    #[serde(default)]
    recommendations: RecommendationsResponse,
}

#[derive(Debug, Default, Deserialize)]
struct ReleaseDatesResponse {
    #[serde(default)]
    results: Vec<ReleaseDatesCountry>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDatesCountry {
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<ReleaseDateEntry>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDateEntry {
    #[serde(default)]
    certification: String,
}

#[derive(Debug, Default, Deserialize)]
struct TvExtrasResponse {
    #[serde(default)]
    videos: VideosResponse,
    #[serde(default)]
    content_ratings: ContentRatingsResponse,
    #[serde(default)]
    recommendations: RecommendationsResponse,
}

#[derive(Debug, Default, Deserialize)]
struct ContentRatingsResponse {
    #[serde(default)]
    results: Vec<ContentRatingCountry>,
}

#[derive(Debug, Deserialize)]
struct ContentRatingCountry {
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

#[derive(Debug, Default, Deserialize)]
struct RecommendationsResponse {
    #[serde(default)]
    results: Vec<RecommendationHit>,
}

#[derive(Debug, Deserialize)]
struct RecommendationHit {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct GenreListResponse {
    #[serde(default)]
    genres: Vec<GenreNameObj>,
}

#[derive(Debug, Deserialize)]
struct GenreNameObj {
    id: i64,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{self, ProviderKind};
    use crate::db::schema::open_connection_at;
    use axum::routing::any;
    use axum::Router;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex as StdMutex;

    // ── similarity / choose_best_match (sin red) ────────────────────────────
    #[test]
    fn similarity_identical_strings_is_one() {
        assert_eq!(similarity("Blade Runner 2049", "Blade Runner 2049"), 1.0);
    }

    #[test]
    fn similarity_is_case_insensitive() {
        assert_eq!(similarity("blade runner", "BLADE RUNNER"), 1.0);
    }

    #[test]
    fn similarity_completely_different_is_low() {
        assert!(similarity("Blade Runner", "Zzzzzzzzzzzz") < 0.3);
    }

    fn hit(title: &str, year: Option<i32>) -> TmdbSearchHit {
        TmdbSearchHit { id: 1, title: title.to_string(), year, overview: None, poster_path: None, backdrop_path: None, rating: None, genre_ids: vec![] }
    }

    #[test]
    fn choose_best_match_accepts_above_threshold_with_matching_year() {
        let results = vec![hit("Blade Runner 2049", Some(2017))];
        assert!(choose_best_match(&results, "Blade Runner 2049", Some(2017)).is_some());
    }

    #[test]
    fn choose_best_match_rejects_below_threshold() {
        let results = vec![hit("Algo Completamente Distinto", Some(2017))];
        assert!(choose_best_match(&results, "Blade Runner 2049", Some(2017)).is_none());
    }

    #[test]
    fn choose_best_match_rejects_year_too_far() {
        let results = vec![hit("Blade Runner 2049", Some(2010))];
        assert!(choose_best_match(&results, "Blade Runner 2049", Some(2017)).is_none());
    }

    #[test]
    fn choose_best_match_accepts_year_off_by_one() {
        let results = vec![hit("Blade Runner 2049", Some(2018))];
        assert!(choose_best_match(&results, "Blade Runner 2049", Some(2017)).is_some());
    }

    #[test]
    fn choose_best_match_skips_year_check_when_missing() {
        let results = vec![hit("Blade Runner 2049", None)];
        assert!(choose_best_match(&results, "Blade Runner 2049", Some(2017)).is_some());
    }

    #[test]
    fn choose_best_match_none_when_no_results() {
        assert!(choose_best_match(&[], "Cualquiera", None).is_none());
    }

    // ── mock server para TmdbClient ──────────────────────────────────────────
    #[derive(Clone, Default)]
    struct MockState {
        responses: Arc<StdMutex<VecDeque<(u16, String)>>>,
        call_count: Arc<AtomicUsize>,
    }

    async fn any_handler(axum::extract::State(state): axum::extract::State<MockState>) -> axum::response::Response {
        state.call_count.fetch_add(1, Ordering::SeqCst);
        let (status, body) = state.responses.lock().unwrap().pop_front().unwrap_or((500, "sin respuesta programada".to_string()));
        axum::response::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    async fn spawn_mock_server(state: MockState) -> String {
        let router = Router::new().fallback(any(any_handler)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    fn queue(state: &MockState, status: u16, body: impl Into<String>) {
        state.responses.lock().unwrap().push_back((status, body.into()));
    }

    #[tokio::test]
    async fn search_movie_parses_results_and_extracts_year() {
        let state = MockState::default();
        queue(
            &state,
            200,
            r#"{"results":[{"id":603,"title":"The Matrix","release_date":"1999-03-30","overview":"o","poster_path":"/p.jpg","backdrop_path":"/b.jpg","vote_average":8.2,"genre_ids":[28,878]}]}"#,
        );
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let hits = client.search_movie("The Matrix", Some(1999), "en-US").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, 603);
        assert_eq!(hits[0].year, Some(1999));
        assert_eq!(hits[0].genre_ids, vec![28, 878]);
    }

    #[tokio::test]
    async fn search_tv_parses_name_and_first_air_date() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[{"id":1399,"name":"Game of Thrones","first_air_date":"2011-04-17","genre_ids":[18]}]}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let hits = client.search_tv("Game of Thrones", None, "en-US").await.unwrap();
        assert_eq!(hits[0].title, "Game of Thrones");
        assert_eq!(hits[0].year, Some(2011));
    }

    #[tokio::test]
    async fn movie_details_parses_genres_and_runtime() {
        let state = MockState::default();
        queue(&state, 200, r#"{"id":603692,"overview":"o","poster_path":"/p.jpg","backdrop_path":"/b.jpg","vote_average":7.8,"genres":[{"id":28,"name":"Action"}],"runtime":169}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let details = client.movie_details(603692, "es-ES").await.unwrap();
        assert_eq!(details.tmdb_id, 603692);
        assert_eq!(details.genre_ids, vec![28]);
        assert_eq!(details.runtime_min, Some(169));
    }

    #[test]
    fn pick_trailer_prefers_official_over_first() {
        let results = vec![
            VideoWire { key: "no-oficial".into(), site: "YouTube".into(), kind: "Trailer".into(), official: false },
            VideoWire { key: "oficial".into(), site: "YouTube".into(), kind: "Trailer".into(), official: true },
        ];
        assert_eq!(pick_trailer(&results), Some("oficial".to_string()));
    }

    #[test]
    fn pick_trailer_ignores_teasers_and_non_youtube_sites() {
        let results = vec![
            VideoWire { key: "teaser".into(), site: "YouTube".into(), kind: "Teaser".into(), official: true },
            VideoWire { key: "vimeo".into(), site: "Vimeo".into(), kind: "Trailer".into(), official: true },
        ];
        assert_eq!(pick_trailer(&results), None);
    }

    #[test]
    fn pick_trailer_falls_back_to_first_trailer_when_none_official() {
        let results = vec![VideoWire { key: "unico".into(), site: "YouTube".into(), kind: "Trailer".into(), official: false }];
        assert_eq!(pick_trailer(&results), Some("unico".to_string()));
    }

    #[test]
    fn pick_trailer_none_when_no_results() {
        assert_eq!(pick_trailer(&[]), None);
    }

    #[tokio::test]
    async fn tv_details_has_no_runtime() {
        let state = MockState::default();
        queue(&state, 200, r#"{"id":1399,"overview":"o","genres":[{"id":18,"name":"Drama"}]}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let details = client.tv_details(1399, "es-ES").await.unwrap();
        assert_eq!(details.runtime_min, None);
        assert_eq!(details.genre_ids, vec![18]);
    }

    #[test]
    fn pick_certification_prefers_spain_over_everything() {
        let entries = vec![("US", Some("PG-13")), ("ES", Some("12")), ("FR", Some("10"))];
        assert_eq!(pick_certification(entries.into_iter()), Some("12".to_string()));
    }

    #[test]
    fn pick_certification_falls_back_to_us_without_spain() {
        let entries = vec![("FR", Some("10")), ("US", Some("PG-13"))];
        assert_eq!(pick_certification(entries.into_iter()), Some("PG-13".to_string()));
    }

    #[test]
    fn pick_certification_falls_back_to_any_country_as_last_resort() {
        let entries = vec![("FR", Some("10"))];
        assert_eq!(pick_certification(entries.into_iter()), Some("10".to_string()));
    }

    #[test]
    fn pick_certification_none_when_every_entry_is_empty() {
        let entries = vec![("ES", None), ("US", None)];
        assert_eq!(pick_certification(entries.into_iter()), None);
    }

    #[tokio::test]
    async fn movie_extras_combines_trailer_and_spanish_certification_in_one_call() {
        let state = MockState::default();
        let call_count = Arc::clone(&state.call_count);
        queue(
            &state,
            200,
            r#"{"id":603,"videos":{"results":[{"key":"abc123","site":"YouTube","type":"Trailer","official":true}]},
                "release_dates":{"results":[{"iso_3166_1":"US","release_dates":[{"certification":"R"}]},
                                             {"iso_3166_1":"ES","release_dates":[{"certification":"16"}]}]},
                "recommendations":{"results":[{"id":604},{"id":605}]}}"#,
        );
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let extras = client.movie_extras(603).await.unwrap();
        assert_eq!(extras.trailer_key, Some("abc123".to_string()));
        assert_eq!(extras.age_rating, Some("16".to_string()));
        assert_eq!(extras.similar_tmdb_ids, vec![604, 605]);
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // una sola llamada, no dos: trailer + certificación + similares juntos.
    }

    #[tokio::test]
    async fn tv_extras_reads_content_ratings() {
        let state = MockState::default();
        queue(
            &state,
            200,
            r#"{"id":1399,"videos":{"results":[]},"content_ratings":{"results":[{"iso_3166_1":"ES","rating":"16"}]},
                "recommendations":{"results":[{"id":1396}]}}"#,
        );
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let extras = client.tv_extras(1399).await.unwrap();
        assert_eq!(extras.trailer_key, None);
        assert_eq!(extras.age_rating, Some("16".to_string()));
        assert_eq!(extras.similar_tmdb_ids, vec![1396]);
    }

    #[tokio::test]
    async fn movie_genres_parses_id_and_name_pairs() {
        let state = MockState::default();
        queue(&state, 200, r#"{"genres":[{"id":28,"name":"Acción"},{"id":18,"name":"Drama"}]}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let genres = client.movie_genres("es-ES").await.unwrap();
        assert_eq!(genres, vec![(28, "Acción".to_string()), (18, "Drama".to_string())]);
    }

    #[tokio::test]
    async fn movie_recommendations_returns_just_the_ids() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[{"id":603},{"id":604},{"id":605}]}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        assert_eq!(client.movie_recommendations(1).await.unwrap(), vec![603, 604, 605]);
    }

    #[tokio::test]
    async fn tv_recommendations_returns_just_the_ids() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[{"id":1396},{"id":1398}]}"#);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        assert_eq!(client.tv_recommendations(1).await.unwrap(), vec![1396, 1398]);
    }

    #[tokio::test]
    async fn retries_on_429_then_succeeds() {
        let state = MockState::default();
        queue(&state, 429, "");
        queue(&state, 200, r#"{"results":[]}"#);
        let call_count = Arc::clone(&state.call_count);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let hits = client.search_movie("X", None, "en-US").await.unwrap();
        assert!(hits.is_empty());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rate_limited_error_after_exhausting_retries() {
        let state = MockState::default();
        for _ in 0..10 {
            queue(&state, 429, "");
        }
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);

        let err = client.search_movie("X", None, "en-US").await.unwrap_err();
        assert!(matches!(err, TmdbError::RateLimited));
    }

    // ── run_matching_pass end-to-end ─────────────────────────────────────────
    fn memory_conn() -> Connection {
        open_connection_at(Path::new(":memory:")).unwrap()
    }

    #[tokio::test]
    async fn run_matching_pass_matches_via_search_and_caches() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[{"id":603,"title":"The Matrix","release_date":"1999-03-30","genre_ids":[28]}]}"#); // es-ES
        let base_url = spawn_mock_server(state).await;

        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie = crate::sources::RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "The Matrix".into(),
            container_extension: None,
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: None,
        };
        let fields = queries::TitleFields { clean_title: "The Matrix", year: Some(1999), match_key: "the-matrix-1999", quality: None, language: None };
        queries::upsert_movie(&conn, provider_id, &std::collections::HashMap::new(), &movie, &fields).unwrap();

        let client = Arc::new(TmdbClient::with_base_url("key", base_url));
        let summary = run_matching_pass(conn, client, Arc::new(AtomicBool::new(false)), Arc::new(|_, _| {})).await.unwrap();

        assert_eq!(summary.total, 1);
        assert_eq!(summary.matched, 1);
    }

    #[tokio::test]
    async fn run_matching_pass_uses_direct_tmdb_id_and_skips_search() {
        let state = MockState::default();
        queue(&state, 200, r#"{"id":603692,"overview":"o","genres":[{"id":28,"name":"Action"}],"runtime":169}"#); // details, no search
        let call_count = Arc::clone(&state.call_count);
        let base_url = spawn_mock_server(state).await;

        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie = crate::sources::RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "John Wick 4".into(),
            container_extension: None,
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: Some(603692), // el proveedor ya lo da
        };
        let fields = queries::TitleFields { clean_title: "John Wick 4", year: Some(2023), match_key: "john-wick-4-2023", quality: None, language: None };
        queries::upsert_movie(&conn, provider_id, &std::collections::HashMap::new(), &movie, &fields).unwrap();

        let client = Arc::new(TmdbClient::with_base_url("key", base_url));
        let summary = run_matching_pass(conn, client, Arc::new(AtomicBool::new(false)), Arc::new(|_, _| {})).await.unwrap();

        assert_eq!(summary.matched, 1);
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // una sola llamada: /movie/{id}, sin búsqueda
    }

    #[tokio::test]
    async fn run_matching_pass_marks_not_found_when_nothing_matches() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[]}"#); // es-ES vacío
        queue(&state, 200, r#"{"results":[]}"#); // en-US también vacío
        let base_url = spawn_mock_server(state).await;

        let conn = memory_conn();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let movie = crate::sources::RawMovie {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_title: "Pelicula Inexistente".into(),
            container_extension: None,
            logo_url: None,
            added_at: None,
            rating: None,
            tmdb_id: None,
        };
        let fields = queries::TitleFields { clean_title: "Pelicula Inexistente", year: None, match_key: "pelicula-inexistente-0000", quality: None, language: None };
        queries::upsert_movie(&conn, provider_id, &std::collections::HashMap::new(), &movie, &fields).unwrap();

        let client = Arc::new(TmdbClient::with_base_url("key", base_url));
        let summary = run_matching_pass(conn, client, Arc::new(AtomicBool::new(false)), Arc::new(|_, _| {})).await.unwrap();

        assert_eq!(summary.matched, 0);
    }

    #[tokio::test]
    async fn cache_hit_avoids_a_second_http_call() {
        let state = MockState::default();
        queue(&state, 200, r#"{"results":[{"id":603,"title":"The Matrix","release_date":"1999-03-30","genre_ids":[28]}]}"#);
        let call_count = Arc::clone(&state.call_count);
        let base_url = spawn_mock_server(state).await;
        let client = TmdbClient::with_base_url("key", base_url);
        let conn = AsyncMutex::new(memory_conn());

        let outcome1 = match_title(&conn, &client, MediaKind::Movie, "The Matrix", Some(1999), None).await;
        assert_eq!(outcome1.state, "matched");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Segunda vez, mismo título/año: debe salir de la caché, sin tocar la red de nuevo.
        let outcome2 = match_title(&conn, &client, MediaKind::Movie, "The Matrix", Some(1999), None).await;
        assert_eq!(outcome2.state, "matched");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }
}
