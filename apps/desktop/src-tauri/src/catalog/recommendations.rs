//! "Recomendado para ti" y "Porque viste X", películas y series, por perfil.
//!
//! Cada cosa que el perfil ha hecho con un título es una señal con un peso
//! ([`seed_weight`]): «Me encanta» y «Me gusta» (valoraciones, 2026-10-22), favorito, vista
//! entera o empezada; lo reciente pesa más (vida media de 45 días) y «No es para mí»
//! resta. Las semillas con más peso piden a TMDB sus recomendaciones, y cada candidato
//! suma lo que le aporta cada semilla según su peso y el puesto que TMDB le da
//! ([`score_candidates`]); luego sube un poco si es de los géneros que más gustan al perfil
//! ([`genre_boost`]). Se descarta lo que el perfil ya ha visto, valorado o tiene en su
//! lista, y todo lo que el proveedor no tiene (se cruza con el catálogo local).
//!
//! Sin clave de TMDB, o sin ninguna señal todavía, sale vacío: no es un error.
//!
//! Cacheado en memoria por perfil. `GET /api/home` nunca espera a un cálculo nuevo: si no
//! hay nada en caché se calcula en segundo plano y esa visita sale sin las filas; si lo
//! hay aunque esté caducado, se sirve mientras se refresca (stale-while-revalidate).
//! Valorar algo marca la caché como caducada ([`invalidate`]).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use crate::catalog::tmdb::{TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
use crate::db::{now_epoch, reads, schema};
use crate::util::keychain;

const CACHE_TTL: Duration = Duration::from_secs(3600);
const MAX_POSITIVE_SEEDS: usize = 6;
const MAX_NEGATIVE_SEEDS: usize = 3;
const MAX_RESULTS: usize = 20;
/// Una fila «Porque viste X» con menos de esto no se enseña.
const MIN_BECAUSE_ROW: usize = 4;
const BECAUSE_ROW_SIZE: usize = 12;
const RECENCY_HALF_LIFE_DAYS: f64 = 45.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RecommendedMovie {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
}

/// Igual que `RecommendedMovie`, para series.
#[derive(Debug, Clone, PartialEq)]
pub struct RecommendedSeries {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
}

/// Un título de una fila «Porque viste X».
#[derive(Debug, Clone, PartialEq)]
pub struct RowTitle {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
}

/// Fila «Porque viste X» (como en Netflix): lo que TMDB relaciona con una de las semillas
/// más fuertes del perfil.
#[derive(Debug, Clone, PartialEq)]
pub struct BecauseRow {
    /// `movie` o `series`.
    pub kind: &'static str,
    pub seed_title: String,
    pub items: Vec<RowTitle>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recommendations {
    pub movies: Vec<RecommendedMovie>,
    pub series: Vec<RecommendedSeries>,
    pub because: Vec<BecauseRow>,
}

struct CacheEntry {
    computed_at: Instant,
    recommendations: Recommendations,
    /// Evita dos cálculos a la vez para el mismo perfil.
    refreshing: bool,
}

static CACHE: LazyLock<AsyncMutex<HashMap<i64, CacheEntry>>> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));

/// Nunca bloquea: lo que haya en caché (vacío si todavía nada) y, si hace falta, un
/// cálculo nuevo por detrás para la próxima visita a Inicio.
pub async fn get_cached_or_refresh(db_path: PathBuf, profile_id: i64) -> Recommendations {
    let mut cache = CACHE.lock().await;
    let (current, should_refresh) = match cache.get_mut(&profile_id) {
        Some(entry) => {
            let should_refresh = entry.computed_at.elapsed() >= CACHE_TTL && !entry.refreshing;
            if should_refresh {
                entry.refreshing = true;
            }
            (entry.recommendations.clone(), should_refresh)
        }
        None => {
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), recommendations: Recommendations::default(), refreshing: true });
            (Recommendations::default(), true)
        }
    };
    drop(cache); // el cálculo puede tardar (varias llamadas a TMDB)

    if should_refresh {
        tokio::spawn(async move {
            let recommendations = compute(db_path, profile_id).await;
            let mut cache = CACHE.lock().await;
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), recommendations, refreshing: false });
        });
    }
    current
}

/// El perfil cambió de gusto (valoró algo): la próxima visita a Inicio recalcula, sin
/// dejar de enseñar mientras tanto lo que había.
pub async fn invalidate(profile_id: i64) {
    let mut cache = CACHE.lock().await;
    if let Some(entry) = cache.get_mut(&profile_id) {
        entry.computed_at = Instant::now().checked_sub(CACHE_TTL).unwrap_or_else(Instant::now);
    }
}

/// Peso de lo que el perfil hizo con un título. Negativo si lo marcó «No es para mí»
/// (eso no caduca); si no, cada señal suma y todo pesa menos cuanto más viejo.
pub fn seed_weight(signal: &reads::TasteSignal, kind: &str, now: i64) -> f64 {
    if signal.rating == Some(-1) {
        return -2.0;
    }
    let mut weight = match signal.rating {
        Some(2) => 3.0,
        Some(1) => 2.0,
        _ => 0.0,
    };
    if signal.favorite {
        weight += 1.5;
    }
    if kind == "series" {
        if signal.watched >= 1.0 {
            weight += 1.0 + 0.1 * signal.watched.min(10.0);
        }
    } else if signal.watched >= 0.7 {
        weight += 1.5;
    } else if signal.watched >= 0.15 {
        weight += 0.6;
    }
    let age_days = (now - signal.last_at).max(0) as f64 / 86_400.0;
    weight * 0.5f64.powf(age_days / RECENCY_HALF_LIFE_DAYS).max(0.2)
}

/// Un candidato de TMDB con su puntuación y la semilla que más le aportó.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate {
    pub tmdb_id: i64,
    pub score: f64,
    pub because: String,
}

/// Puntúa los candidatos: cada semilla (peso, título, recomendaciones de TMDB en orden)
/// aporta `peso / (1 + 0,15 · puesto)`; las negativas restan. De más a menos puntuación,
/// sin los que acaban en cero o menos.
pub fn score_candidates(seeds: &[(f64, String, Vec<i64>)]) -> Vec<ScoredCandidate> {
    let mut order: Vec<i64> = Vec::new();
    let mut scores: HashMap<i64, (f64, f64, String)> = HashMap::new(); // puntuación, mejor aporte, de quién
    for (weight, title, recommendations) in seeds {
        for (rank, tmdb_id) in recommendations.iter().enumerate() {
            let contribution = weight / (1.0 + 0.15 * rank as f64);
            let entry = scores.entry(*tmdb_id).or_insert_with(|| {
                order.push(*tmdb_id);
                (0.0, 0.0, String::new())
            });
            entry.0 += contribution;
            if contribution > entry.1 {
                entry.1 = contribution;
                entry.2 = title.clone();
            }
        }
    }
    let mut scored: Vec<ScoredCandidate> = order
        .into_iter()
        .filter_map(|tmdb_id| {
            let (score, _, because) = scores.remove(&tmdb_id)?;
            (score > 0.0).then_some(ScoredCandidate { tmdb_id, score, because })
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

/// Cuánto gusta cada género al perfil, de 0 a 1, a partir de sus semillas positivas.
pub fn genre_affinity(seeds: &[(f64, &[i64])]) -> HashMap<i64, f64> {
    let mut affinity: HashMap<i64, f64> = HashMap::new();
    for (weight, genres) in seeds {
        if *weight <= 0.0 {
            continue;
        }
        for genre in *genres {
            *affinity.entry(*genre).or_default() += weight;
        }
    }
    let max = affinity.values().cloned().fold(0.0, f64::max);
    if max > 0.0 {
        affinity.values_mut().for_each(|v| *v /= max);
    }
    affinity
}

/// Multiplicador de un candidato por sus géneros: entre 1 y 1,3.
pub fn genre_boost(affinity: &HashMap<i64, f64>, genres: &[i64]) -> f64 {
    let best = genres.iter().filter_map(|g| affinity.get(g)).cloned().fold(0.0, f64::max);
    1.0 + 0.3 * best
}

struct KindResult {
    ranked: Vec<(reads::LocalTitle, String)>,
    /// Semilla positiva más fuerte con su fila, si salió bastante.
    because: Option<(f64, BecauseRow)>,
}

async fn compute(db_path: PathBuf, profile_id: i64) -> Recommendations {
    let Ok(api_key) = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF) else { return Recommendations::default() };
    let client = TmdbClient::new(api_key);
    let movies = compute_kind(&client, &db_path, profile_id, "movie").await;
    let series = compute_kind(&client, &db_path, profile_id, "series").await;

    let mut because: Vec<(f64, BecauseRow)> = movies.as_ref().and_then(|r| r.because.clone()).into_iter().chain(series.as_ref().and_then(|r| r.because.clone())).collect();
    because.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    Recommendations {
        movies: movies
            .map(|r| r.ranked.into_iter().map(|(t, because_of)| RecommendedMovie { id: t.id, clean_title: t.clean_title, year: t.year, poster_path: t.poster_path, rating: t.rating, because_of }).collect())
            .unwrap_or_default(),
        series: series
            .map(|r| r.ranked.into_iter().map(|(t, because_of)| RecommendedSeries { id: t.id, clean_title: t.clean_title, year: t.year, poster_path: t.poster_path, rating: t.rating, because_of }).collect())
            .unwrap_or_default(),
        because: because.into_iter().map(|(_, row)| row).collect(),
    }
}

async fn compute_kind(client: &TmdbClient, db_path: &PathBuf, profile_id: i64, kind: &'static str) -> Option<KindResult> {
    let signals = {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = schema::open_readonly_at(&db_path)?;
            reads::list_taste_signals(&conn, profile_id, kind)
        })
        .await
        .ok()?
        .ok()?
    };
    let now = now_epoch();
    let mut weighted: Vec<(f64, &reads::TasteSignal)> =
        signals.iter().filter(|s| s.tmdb_id.is_some()).map(|s| (seed_weight(s, kind, now), s)).filter(|(w, _)| *w != 0.0).collect();
    weighted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let positives: Vec<(f64, &reads::TasteSignal)> = weighted.iter().filter(|(w, _)| *w > 0.0).take(MAX_POSITIVE_SEEDS).cloned().collect();
    if positives.is_empty() {
        return None;
    }
    let mut negatives: Vec<(f64, &reads::TasteSignal)> = weighted.iter().filter(|(w, _)| *w < 0.0).cloned().collect();
    negatives.sort_by_key(|(_, s)| std::cmp::Reverse(s.last_at));
    negatives.truncate(MAX_NEGATIVE_SEEDS);

    let mut seeds: Vec<(f64, String, Vec<i64>)> = Vec::new();
    for (weight, signal) in positives.iter().chain(negatives.iter()) {
        let Some(tmdb_id) = signal.tmdb_id else { continue };
        let recommendations = if kind == "series" { client.tv_recommendations(tmdb_id).await } else { client.movie_recommendations(tmdb_id).await };
        if let Ok(recommendations) = recommendations {
            seeds.push((*weight, signal.clean_title.clone(), recommendations));
        }
    }
    let scored = score_candidates(&seeds);
    let wanted: Vec<i64> = scored.iter().map(|c| c.tmdb_id).collect();
    let local = {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = schema::open_readonly_at(&db_path)?;
            reads::local_titles_by_tmdb(&conn, kind, &wanted)
        })
        .await
        .ok()?
        .ok()?
    };

    // Lo que ya conoce (visto, empezado, valorado o en su lista) no se le recomienda.
    let known: HashSet<i64> = signals.iter().map(|s| s.id).collect();
    let seed_genres: Vec<(f64, &[i64])> = positives.iter().map(|(w, s)| (*w, s.genres.as_slice())).collect();
    let affinity = genre_affinity(&seed_genres);
    let mut ranked: Vec<(f64, reads::LocalTitle, String)> = scored
        .iter()
        .filter_map(|c| {
            let title = local.get(&c.tmdb_id)?;
            (!known.contains(&title.id)).then(|| (c.score * genre_boost(&affinity, &title.genres), title.clone(), c.because.clone()))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = HashSet::new();
    ranked.retain(|(_, title, _)| seen.insert(title.id));

    // «Porque viste X»: lo de la semilla positiva más fuerte, en el orden de TMDB.
    let because = positives.first().and_then(|(weight, signal)| {
        let (_, _, recommendations) = seeds.iter().find(|(_, title, _)| *title == signal.clean_title)?;
        let mut used = HashSet::new();
        let items: Vec<RowTitle> = recommendations
            .iter()
            .filter_map(|id| local.get(id))
            .filter(|t| !known.contains(&t.id) && used.insert(t.id))
            .take(BECAUSE_ROW_SIZE)
            .map(|t| RowTitle { id: t.id, clean_title: t.clean_title.clone(), year: t.year, poster_path: t.poster_path.clone() })
            .collect();
        (items.len() >= MIN_BECAUSE_ROW).then(|| (*weight, BecauseRow { kind, seed_title: signal.clean_title.clone(), items }))
    });

    Some(KindResult { ranked: ranked.into_iter().take(MAX_RESULTS).map(|(_, t, because)| (t, because)).collect(), because })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn signal(rating: Option<i64>, favorite: bool, watched: f64, last_at: i64) -> reads::TasteSignal {
        reads::TasteSignal { id: 1, clean_title: "X".into(), tmdb_id: Some(1), rating, favorite, watched, last_at, genres: vec![] }
    }

    #[test]
    fn loving_weighs_more_than_liking_and_more_than_just_watching() {
        let now = 100 * DAY;
        let loved = seed_weight(&signal(Some(2), false, 0.0, now), "movie", now);
        let liked = seed_weight(&signal(Some(1), false, 0.0, now), "movie", now);
        let watched = seed_weight(&signal(None, false, 1.0, now), "movie", now);
        let started = seed_weight(&signal(None, false, 0.3, now), "movie", now);
        assert!(loved > liked && liked > watched && watched > started && started > 0.0);
    }

    #[test]
    fn a_dislike_always_subtracts_even_if_it_was_watched_long_ago() {
        let now = 1000 * DAY;
        assert!(seed_weight(&signal(Some(-1), true, 1.0, 0), "movie", now) < 0.0);
    }

    #[test]
    fn recent_tastes_weigh_more_than_old_ones() {
        let now = 400 * DAY;
        let recent = seed_weight(&signal(Some(1), false, 0.0, now - DAY), "movie", now);
        let old = seed_weight(&signal(Some(1), false, 0.0, now - 300 * DAY), "movie", now);
        assert!(recent > old && old > 0.0, "lo viejo pesa menos pero sigue contando");
    }

    #[test]
    fn candidates_recommended_by_several_seeds_and_near_the_top_rank_first() {
        let seeds = vec![
            (2.0, "A".to_string(), vec![10, 11, 12]),
            (2.0, "B".to_string(), vec![12, 13]),
        ];
        let scored = score_candidates(&seeds);
        assert_eq!(scored[0].tmdb_id, 12, "lo recomiendan dos semillas");
        assert_eq!(scored[1].tmdb_id, 10);
        assert_eq!(scored.iter().find(|c| c.tmdb_id == 13).unwrap().because, "B");
    }

    #[test]
    fn what_a_disliked_title_suggests_sinks_or_disappears() {
        let seeds = vec![(1.0, "Me gusta".to_string(), vec![20, 21]), (-2.0, "No es para mí".to_string(), vec![21, 22])];
        let scored = score_candidates(&seeds);
        let ids: Vec<i64> = scored.iter().map(|c| c.tmdb_id).collect();
        assert_eq!(ids, vec![20], "21 se anula y 22 solo resta");
    }

    #[test]
    fn favourite_genres_give_a_small_boost() {
        let action: &[i64] = &[28];
        let drama: &[i64] = &[18];
        let affinity = genre_affinity(&[(3.0, action), (1.0, drama), (-2.0, &[99])]);
        assert_eq!(genre_boost(&affinity, &[28]), 1.3);
        assert!(genre_boost(&affinity, &[18]) > 1.0 && genre_boost(&affinity, &[18]) < 1.3);
        assert_eq!(genre_boost(&affinity, &[99]), 1.0, "lo que no gusta no suma");
    }
}
