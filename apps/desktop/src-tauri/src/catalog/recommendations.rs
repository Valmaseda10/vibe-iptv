//! "Recomendado para ti" (F "premium", informe de I+D): a partir de lo que el perfil
//! marcó como favorito o vio hasta pasado el 70%, se piden sugerencias a TMDB y se
//! cruzan con el catálogo local — recomendar algo que el proveedor no tiene sería el
//! error clásico que señala el informe. Sin clave de TMDB configurada, o sin ninguna
//! película semilla todavía, la lista sale vacía: no es un error, solo no hay con qué
//! trabajar todavía.
//!
//! Cacheado en memoria por perfil (no en SQLite: es un resultado derivado, fácil de
//! recalcular, y así una reinstalación o un perfil borrado no deja basura que limpiar).
//! `GET /api/home` nunca espera a un cálculo nuevo: si no hay nada en caché todavía, se
//! dispara el cálculo en segundo plano y esa visita a Inicio sale sin la fila — la
//! siguiente ya la tiene. Si hay algo cacheado aunque esté caducado, se sirve tal cual
//! mientras se refresca por detrás (stale-while-revalidate), para que la fila no
//! parpadee vacía cada hora.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use crate::catalog::tmdb::{TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
use crate::db::{reads, schema};
use crate::util::keychain;

const CACHE_TTL: Duration = Duration::from_secs(3600);
const MAX_SEEDS: i64 = 5;
const MAX_RECOMMENDATIONS_PER_SEED: usize = 8;
const MAX_RESULTS: i64 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct RecommendedMovie {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
}

impl From<reads::RecommendedMovieRow> for RecommendedMovie {
    fn from(row: reads::RecommendedMovieRow) -> Self {
        Self { id: row.id, clean_title: row.clean_title, year: row.year, poster_path: row.poster_path, rating: row.rating, because_of: row.because_of }
    }
}

struct CacheEntry {
    computed_at: Instant,
    movies: Vec<RecommendedMovie>,
    /// Evita disparar dos cálculos en segundo plano a la vez para el mismo perfil si
    /// llegan varias peticiones de Inicio casi juntas mientras el primero aún corre.
    refreshing: bool,
}

static CACHE: LazyLock<AsyncMutex<HashMap<i64, CacheEntry>>> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));

/// Nunca bloquea: devuelve lo que haya en caché al instante (vacío si todavía no hay
/// nada) y, si hace falta, deja un cálculo nuevo corriendo por detrás para la próxima
/// visita a Inicio.
pub async fn get_cached_or_refresh(db_path: PathBuf, profile_id: i64) -> Vec<RecommendedMovie> {
    let mut cache = CACHE.lock().await;
    let entry = cache.get_mut(&profile_id);

    let (movies, should_refresh) = match entry {
        Some(entry) => {
            let stale = entry.computed_at.elapsed() >= CACHE_TTL;
            let should_refresh = stale && !entry.refreshing;
            if should_refresh {
                entry.refreshing = true;
            }
            (entry.movies.clone(), should_refresh)
        }
        None => {
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), movies: Vec::new(), refreshing: true });
            (Vec::new(), true)
        }
    };
    drop(cache); // el cálculo en sí puede tardar (varias llamadas a TMDB): no reteniendo el lock.

    if should_refresh {
        tokio::spawn(async move {
            let computed = compute(db_path, profile_id).await.unwrap_or_default();
            let mut cache = CACHE.lock().await;
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), movies: computed, refreshing: false });
        });
    }

    movies
}

async fn compute(db_path: PathBuf, profile_id: i64) -> Option<Vec<RecommendedMovie>> {
    let api_key = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).ok()?;
    let client = TmdbClient::new(api_key);

    let seeds = {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = schema::open_readonly_at(&db_path)?;
            reads::list_recommendation_seed_movies(&conn, profile_id, MAX_SEEDS)
        })
        .await
        .ok()?
        .ok()?
    };
    if seeds.is_empty() {
        return None;
    }

    let mut candidates: Vec<(i64, String)> = Vec::new();
    for seed in &seeds {
        if let Ok(hits) = client.movie_recommendations(seed.tmdb_id).await {
            for tmdb_id in hits.into_iter().take(MAX_RECOMMENDATIONS_PER_SEED) {
                candidates.push((tmdb_id, seed.clean_title.clone()));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let exclude: Vec<i64> = seeds.iter().map(|s| s.id).collect();
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path).ok()?;
        reads::resolve_recommendations(&conn, &candidates, &exclude, MAX_RESULTS).ok()
    })
    .await
    .ok()?
    .map(|rows| rows.into_iter().map(Into::into).collect())
}
