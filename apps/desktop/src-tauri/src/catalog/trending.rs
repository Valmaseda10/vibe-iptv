//! "Tendencias ahora" (F "premium"): lo más visto/comentado de TMDB esta semana, igual
//! para cualquier perfil — a diferencia de `catalog::recommendations`, esto no depende
//! de nada que se haya visto en esta app, es una lista externa que TMDB va actualizando
//! sola. Se cruza contra el catálogo local con la misma regla de oro de siempre: nunca
//! enseñar un título que el proveedor no tiene.
//!
//! Caché única en memoria (no por perfil: la propia lista de TMDB ya es la misma para
//! todo el mundo), con una vida más larga que la de `recommendations` — TMDB solo la
//! recalcula ellos una vez al día, refrescarla cada hora sería tirar peticiones a la
//! basura. Mismo patrón de "nunca bloquear, refrescar por detrás" que el resto de
//! `catalog::*`.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use crate::catalog::tmdb::{TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
use crate::db::{reads, schema};
use crate::util::keychain;

const CACHE_TTL: Duration = Duration::from_secs(6 * 3600);
const MAX_RESULTS: i64 = 20;
/// Ningún id real de la base de datos es 0 (`INTEGER PRIMARY KEY` empieza en 1): sirve
/// para reutilizar `resolve_similar_movies`/`resolve_similar_series` sin tener que
/// excluir ningún título de verdad, que es justo lo que hace falta aquí (no hay ningún
/// "título de origen" del que huir, a diferencia de "Más como esto").
const NOTHING_TO_EXCLUDE: i64 = 0;

#[derive(Debug, Clone, PartialEq)]
pub struct TrendingTitle {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
}

impl From<reads::SimilarTitleRow> for TrendingTitle {
    fn from(row: reads::SimilarTitleRow) -> Self {
        Self { id: row.id, clean_title: row.clean_title, year: row.year, poster_path: row.poster_path }
    }
}

struct CacheEntry {
    computed_at: Instant,
    movies: Vec<TrendingTitle>,
    series: Vec<TrendingTitle>,
    refreshing: bool,
}

static CACHE: LazyLock<AsyncMutex<Option<CacheEntry>>> = LazyLock::new(|| AsyncMutex::new(None));

/// Nunca bloquea: sirve lo que haya en caché (vacío la primerísima vez) y deja un
/// refresco corriendo por detrás si hace falta.
pub async fn get_cached_or_refresh(db_path: PathBuf) -> (Vec<TrendingTitle>, Vec<TrendingTitle>) {
    let mut cache = CACHE.lock().await;
    let (movies, series, should_refresh) = match cache.as_mut() {
        Some(entry) => {
            let stale = entry.computed_at.elapsed() >= CACHE_TTL;
            let should_refresh = stale && !entry.refreshing;
            if should_refresh {
                entry.refreshing = true;
            }
            (entry.movies.clone(), entry.series.clone(), should_refresh)
        }
        None => {
            *cache = Some(CacheEntry { computed_at: Instant::now(), movies: Vec::new(), series: Vec::new(), refreshing: true });
            (Vec::new(), Vec::new(), true)
        }
    };
    drop(cache);

    if should_refresh {
        tokio::spawn(async move {
            let (movies, series) = compute(db_path).await.unwrap_or_default();
            let mut cache = CACHE.lock().await;
            *cache = Some(CacheEntry { computed_at: Instant::now(), movies, series, refreshing: false });
        });
    }

    (movies, series)
}

async fn compute(db_path: PathBuf) -> Option<(Vec<TrendingTitle>, Vec<TrendingTitle>)> {
    let api_key = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).ok()?;
    let client = TmdbClient::new(api_key);

    // Cada lista es independiente: si una de las dos peticiones a TMDB falla, la otra
    // igual puede servir algo en vez de quedarse las dos a cero.
    let movie_ids = client.trending_movies().await.unwrap_or_default();
    let series_ids = client.trending_series().await.unwrap_or_default();
    if movie_ids.is_empty() && series_ids.is_empty() {
        return None;
    }

    let movies = {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = schema::open_readonly_at(&db_path).ok()?;
            reads::resolve_similar_movies(&conn, &movie_ids, NOTHING_TO_EXCLUDE, MAX_RESULTS).ok()
        })
        .await
        .ok()?
        .unwrap_or_default()
    };
    let series = tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path).ok()?;
        reads::resolve_similar_series(&conn, &series_ids, NOTHING_TO_EXCLUDE, MAX_RESULTS).ok()
    })
    .await
    .ok()?
    .unwrap_or_default();

    Some((movies.into_iter().map(Into::into).collect(), series.into_iter().map(Into::into).collect()))
}
