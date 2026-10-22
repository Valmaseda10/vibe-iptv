//! "Recomendado para ti" y "Porque viste X", películas y series, por perfil.
//!
//! Cada cosa que el perfil ha hecho con un título es una señal con un peso
//! ([`seed_weight`]): «Me encanta» y «Me gusta» (valoraciones, 2026-10-22), favorito, vista
//! entera o empezada; lo reciente pesa más (vida media de 45 días). «No es para mí» resta,
//! y algo empezado y dejado enseguida resta un poco (abandonado). Las semillas con más peso
//! piden a TMDB sus recomendaciones, y cada candidato suma lo que le aporta cada semilla
//! según su peso y el puesto que TMDB le da ([`score_candidates`]). Después:
//! - sube si es de los géneros que más gustan al perfil y baja si es de los que no
//!   ([`genre_boost`]);
//! - sube un poco cuanto mejor valorado está en TMDB ([`quality_factor`]);
//! - se reparte entre semillas, para que una sola no llene la fila ([`diversify`]);
//! - si TMDB da pocas que el proveedor tenga, se completa con lo mejor valorado de los
//!   géneros favoritos del perfil.
//!
//! Se descarta lo que el perfil ya ha visto, valorado o tiene en su lista, y todo lo que el
//! proveedor no tiene (se cruza con el catálogo local). Sin clave de TMDB, o sin ninguna
//! señal todavía, sale vacío: no es un error.
//!
//! Cacheado en memoria por perfil. Las pantallas nunca esperan a un cálculo nuevo: si no
//! hay nada en caché se calcula en segundo plano y esa visita sale sin las filas; si lo
//! hay aunque esté caducado, se sirve mientras se refresca (stale-while-revalidate).
//! Valorar, cambiar la lista o terminar algo marca la caché como caducada ([`invalidate`]).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use crate::catalog::tmdb::{TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
use crate::db::{now_epoch, reads, schema};
use crate::util::keychain;

const CACHE_TTL: Duration = Duration::from_secs(3600);
/// Tras un cambio de gustos, como mucho un recálculo por minuto (cada uno son unas cuantas
/// llamadas a TMDB).
const MIN_RECOMPUTE: Duration = Duration::from_secs(60);
const MAX_POSITIVE_SEEDS: usize = 8;
const MAX_NEGATIVE_SEEDS: usize = 3;
/// Recomendadas que se guardan: las pantallas eligen de aquí unas pocas cada visita.
const MAX_RESULTS: usize = 40;
/// Una fila «Porque viste X» con menos de esto no se enseña.
const MIN_BECAUSE_ROW: usize = 4;
const BECAUSE_ROW_SIZE: usize = 16;
/// Filas «Porque viste X» por tipo, de las semillas más fuertes.
const MAX_BECAUSE_ROWS: usize = 4;
/// Algo empezado y dejado antes de esta fracción, hace más de unos días, cuenta como
/// abandonado.
const ABANDONED_FRACTION: f64 = 0.1;
const ABANDONED_AFTER_DAYS: f64 = 3.0;
const RECENCY_HALF_LIFE_DAYS: f64 = 45.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RecommendedMovie {
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub rating: Option<f64>,
    pub because_of: String,
    /// Por qué, con lo que hizo el perfil: «Porque te gustó X», «Porque viste X»...
    pub reason: String,
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
    /// Igual que en `RecommendedMovie`.
    pub reason: String,
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
    /// Título de la fila, con lo que hizo el perfil con la semilla («Porque viste X»).
    pub row_title: String,
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
    /// El perfil cambió de gustos desde el último cálculo.
    stale: bool,
}

static CACHE: LazyLock<AsyncMutex<HashMap<i64, CacheEntry>>> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));

/// Nunca bloquea: lo que haya en caché (vacío si todavía nada) y, si hace falta, un
/// cálculo nuevo por detrás para la próxima visita a Inicio.
pub async fn get_cached_or_refresh(db_path: PathBuf, profile_id: i64) -> Recommendations {
    let mut cache = CACHE.lock().await;
    let (current, should_refresh) = match cache.get_mut(&profile_id) {
        Some(entry) => {
            let age = entry.computed_at.elapsed();
            let should_refresh = !entry.refreshing && (age >= CACHE_TTL || (entry.stale && age >= MIN_RECOMPUTE));
            if should_refresh {
                entry.refreshing = true;
            }
            (entry.recommendations.clone(), should_refresh)
        }
        None => {
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), recommendations: Recommendations::default(), refreshing: true, stale: false });
            (Recommendations::default(), true)
        }
    };
    drop(cache); // el cálculo puede tardar (varias llamadas a TMDB)

    if should_refresh {
        tokio::spawn(async move {
            let recommendations = compute(db_path, profile_id).await;
            let mut cache = CACHE.lock().await;
            cache.insert(profile_id, CacheEntry { computed_at: Instant::now(), recommendations, refreshing: false, stale: false });
        });
    }
    current
}

/// El catálogo cambió (p. ej. un emparejamiento con TMDB encontró títulos nuevos): todos
/// los perfiles recalculan en su próxima visita.
pub async fn invalidate_all() {
    let mut cache = CACHE.lock().await;
    for entry in cache.values_mut() {
        entry.stale = true;
    }
}

/// Se está calculando la primera vez para este perfil (todavía no hay nada que enseñar).
pub async fn first_computation_pending(profile_id: i64) -> bool {
    let cache = CACHE.lock().await;
    cache.get(&profile_id).is_some_and(|entry| entry.refreshing && entry.recommendations == Recommendations::default())
}

/// El perfil cambió de gusto (valoró algo, cambió su lista o terminó algo): la próxima
/// visita recalcula, sin dejar de enseñar mientras tanto lo que había.
pub async fn invalidate(profile_id: i64) {
    let mut cache = CACHE.lock().await;
    if let Some(entry) = cache.get_mut(&profile_id) {
        entry.stale = true;
    }
}

/// Peso de lo que el perfil hizo con un título. Negativo si lo marcó «No es para mí»
/// (eso no caduca); si no, cada señal suma y todo pesa menos cuanto más viejo.
pub fn seed_weight(signal: &reads::TasteSignal, kind: &str, now: i64) -> f64 {
    if signal.rating == Some(-1) {
        return -2.0;
    }
    let age_days = (now - signal.last_at).max(0) as f64 / 86_400.0;
    // Empezada y dejada enseguida, sin valorar ni guardar: no le convenció.
    if kind != "series" && signal.rating.is_none() && !signal.favorite && signal.watched > 0.0 && signal.watched < ABANDONED_FRACTION && age_days >= ABANDONED_AFTER_DAYS {
        return -0.6;
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

/// Cuánto disgusta cada género al perfil, de 0 a 1, a partir de sus semillas negativas.
pub fn genre_aversion(seeds: &[(f64, &[i64])]) -> HashMap<i64, f64> {
    let negated: Vec<(f64, &[i64])> = seeds.iter().filter(|(w, _)| *w < 0.0).map(|(w, g)| (-w, *g)).collect();
    genre_affinity(&negated)
}

/// Multiplicador de un candidato por sus géneros: hasta 1,3 si es de los que gustan, y
/// hasta 0,65 si es de los que no.
pub fn genre_boost(affinity: &HashMap<i64, f64>, aversion: &HashMap<i64, f64>, genres: &[i64]) -> f64 {
    let best = genres.iter().filter_map(|g| affinity.get(g)).cloned().fold(0.0, f64::max);
    let worst = genres.iter().filter_map(|g| aversion.get(g)).cloned().fold(0.0, f64::max);
    (1.0 + 0.3 * best) * (1.0 - 0.35 * worst)
}

/// Multiplicador por la nota de TMDB: 1 con un 6,5, hasta 1,15 (excelente) y como poco
/// 0,8 (floja); sin nota, 0,95.
pub fn quality_factor(rating: Option<f64>) -> f64 {
    rating.map(|r| (1.0 + 0.075 * (r - 6.5)).clamp(0.8, 1.15)).unwrap_or(0.95)
}

/// Reordena para que no salgan muchas seguidas por la misma semilla: cada vez que una
/// semilla ya ha puesto una, la siguiente suya vale un 15 % menos.
pub fn diversify<T: Clone>(ranked: Vec<(f64, T, String)>) -> Vec<(f64, T, String)> {
    let mut pending = ranked;
    let mut out = Vec::with_capacity(pending.len());
    let mut used: HashMap<String, i32> = HashMap::new();
    while !pending.is_empty() {
        let best = pending
            .iter()
            .enumerate()
            .map(|(i, (score, _, because))| (i, score * 0.85f64.powi(*used.get(because).unwrap_or(&0))))
            .fold((0, f64::MIN), |acc, item| if item.1 > acc.1 { item } else { acc })
            .0;
        let item = pending.remove(best);
        *used.entry(item.2.clone()).or_default() += 1;
        out.push(item);
    }
    out
}

/// Qué hizo el perfil con una semilla, para decir el porqué con verdad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedReason {
    Liked,
    Watched,
    Listed,
    Started,
}

impl SeedReason {
    /// Lo más fuerte que hizo: valorarla, verla (una película casi entera o algún episodio
    /// de una serie), guardarla en su lista o empezarla.
    pub fn of(signal: &reads::TasteSignal, kind: &str) -> Self {
        if matches!(signal.rating, Some(1) | Some(2)) {
            Self::Liked
        } else if (kind == "series" && signal.watched >= 1.0) || (kind != "series" && signal.watched >= 0.7) {
            Self::Watched
        } else if signal.favorite {
            Self::Listed
        } else {
            Self::Started
        }
    }

    pub fn phrase(self, title: &str) -> String {
        match self {
            Self::Liked => format!("Porque te gustó {title}"),
            Self::Watched => format!("Porque viste {title}"),
            Self::Listed => format!("Porque tienes {title} en tu lista"),
            Self::Started => format!("Porque empezaste {title}"),
        }
    }
}

struct KindResult {
    ranked: Vec<(reads::LocalTitle, String)>,
    /// Qué hizo el perfil con cada semilla (por título).
    reasons: HashMap<String, SeedReason>,
    /// Filas «Porque viste X» de las semillas positivas más fuertes, con su peso.
    because: Vec<(f64, BecauseRow)>,
}

async fn compute(db_path: PathBuf, profile_id: i64) -> Recommendations {
    let Ok(api_key) = keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF) else { return Recommendations::default() };
    let client = TmdbClient::new(api_key);
    let movies = compute_kind(&client, &db_path, profile_id, "movie").await;
    let series = compute_kind(&client, &db_path, profile_id, "series").await;

    let mut because: Vec<(f64, BecauseRow)> =
        movies.as_ref().map(|r| r.because.clone()).unwrap_or_default().into_iter().chain(series.as_ref().map(|r| r.because.clone()).unwrap_or_default()).collect();
    because.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    Recommendations {
        movies: movies
            .map(|r| {
                let reasons = r.reasons;
                r.ranked
                    .into_iter()
                    .map(|(t, because_of)| {
                        let reason = reasons.get(&because_of).copied().unwrap_or(SeedReason::Liked).phrase(&because_of);
                        RecommendedMovie { id: t.id, clean_title: t.clean_title, year: t.year, poster_path: t.poster_path, rating: t.rating, because_of, reason }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        series: series
            .map(|r| {
                let reasons = r.reasons;
                r.ranked
                    .into_iter()
                    .map(|(t, because_of)| {
                        let reason = reasons.get(&because_of).copied().unwrap_or(SeedReason::Liked).phrase(&because_of);
                        RecommendedSeries { id: t.id, clean_title: t.clean_title, year: t.year, poster_path: t.poster_path, rating: t.rating, because_of, reason }
                    })
                    .collect()
            })
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
    let all_genres: Vec<(f64, &[i64])> = weighted.iter().map(|(w, s)| (*w, s.genres.as_slice())).collect();
    let affinity = genre_affinity(&all_genres);
    let aversion = genre_aversion(&all_genres);
    let mut ranked: Vec<(f64, reads::LocalTitle, String)> = scored
        .iter()
        .filter_map(|c| {
            let title = local.get(&c.tmdb_id)?;
            (!known.contains(&title.id)).then(|| (c.score * genre_boost(&affinity, &aversion, &title.genres) * quality_factor(title.rating), title.clone(), c.because.clone()))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = HashSet::new();
    ranked.retain(|(_, title, _)| seen.insert(title.id));

    // TMDB recomienda mucho que el proveedor no tiene: si quedan pocas, se completa con lo
    // mejor valorado de los géneros favoritos, atribuido a la semilla de ese género.
    if ranked.len() < MAX_RESULTS {
        let mut favourite_genres: Vec<(i64, f64)> = affinity.iter().map(|(g, a)| (*g, *a)).collect();
        favourite_genres.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        let genres: Vec<i64> = favourite_genres.iter().take(3).map(|(g, _)| *g).collect();
        let fallback = {
            let db_path = db_path.clone();
            let genres = genres.clone();
            tokio::task::spawn_blocking(move || {
                let conn = schema::open_readonly_at(&db_path)?;
                reads::top_rated_in_genres(&conn, kind, &genres, 6.8, 200)
            })
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default()
        };
        let lowest = ranked.last().map(|(score, _, _)| *score).unwrap_or(1.0);
        for title in fallback {
            if ranked.len() >= MAX_RESULTS {
                break;
            }
            if known.contains(&title.id) || !seen.insert(title.id) || aversion.keys().any(|g| title.genres.contains(g) && aversion[g] > 0.5) {
                continue;
            }
            let because = positives
                .iter()
                .find(|(_, s)| s.genres.iter().any(|g| title.genres.contains(g)))
                .or(positives.first())
                .map(|(_, s)| s.clean_title.clone())
                .unwrap_or_default();
            let score = lowest * 0.9 * genre_boost(&affinity, &aversion, &title.genres) * quality_factor(title.rating);
            ranked.push((score, title, because));
        }
    }
    let ranked = diversify(ranked);

    // «Porque viste X»: lo de cada una de las semillas positivas más fuertes, en el orden de TMDB.
    let mut because = Vec::new();
    for (weight, signal) in positives.iter().take(MAX_BECAUSE_ROWS) {
        let Some((_, _, recommendations)) = seeds.iter().find(|(_, title, _)| *title == signal.clean_title) else { continue };
        let mut used = HashSet::new();
        let items: Vec<RowTitle> = recommendations
            .iter()
            .filter_map(|id| local.get(id))
            .filter(|t| !known.contains(&t.id) && used.insert(t.id))
            .take(BECAUSE_ROW_SIZE)
            .map(|t| RowTitle { id: t.id, clean_title: t.clean_title.clone(), year: t.year, poster_path: t.poster_path.clone() })
            .collect();
        if items.len() >= MIN_BECAUSE_ROW {
            let row_title = SeedReason::of(signal, kind).phrase(&signal.clean_title);
            because.push((*weight, BecauseRow { kind, seed_title: signal.clean_title.clone(), row_title, items }));
        }
    }

    let reasons = positives.iter().map(|(_, s)| (s.clean_title.clone(), SeedReason::of(s, kind))).collect();
    Some(KindResult { ranked: ranked.into_iter().take(MAX_RESULTS).map(|(_, t, because)| (t, because)).collect(), reasons, because })
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
    fn favourite_genres_give_a_small_boost_and_disliked_ones_sink() {
        let action: &[i64] = &[28];
        let drama: &[i64] = &[18];
        let horror: &[i64] = &[27];
        let seeds = [(3.0, action), (1.0, drama), (-2.0, horror)];
        let affinity = genre_affinity(&seeds);
        let aversion = genre_aversion(&seeds);
        assert_eq!(genre_boost(&affinity, &aversion, &[28]), 1.3);
        assert!(genre_boost(&affinity, &aversion, &[18]) > 1.0 && genre_boost(&affinity, &aversion, &[18]) < 1.3);
        assert_eq!(genre_boost(&affinity, &aversion, &[27]), 0.65, "lo que no gusta resta");
        assert_eq!(genre_boost(&affinity, &aversion, &[99]), 1.0);
    }

    #[test]
    fn something_started_and_dropped_days_ago_counts_slightly_against() {
        let now = 100 * DAY;
        assert!(seed_weight(&signal(None, false, 0.05, now - 5 * DAY), "movie", now) < 0.0);
        assert_eq!(seed_weight(&signal(None, false, 0.05, now - DAY), "movie", now), 0.0, "de ayer: aún puede volver a ella");
        assert!(seed_weight(&signal(Some(1), false, 0.05, now - 5 * DAY), "movie", now) > 0.0, "si la valoró, manda la valoración");
    }

    #[test]
    fn the_reason_says_what_the_profile_actually_did() {
        let listed = signal(None, true, 0.0, 0);
        assert_eq!(SeedReason::of(&listed, "series").phrase("La Casa de Papel"), "Porque tienes La Casa de Papel en tu lista");
        assert_eq!(SeedReason::of(&signal(Some(2), true, 0.0, 0), "movie"), SeedReason::Liked);
        assert_eq!(SeedReason::of(&signal(None, true, 0.9, 0), "movie"), SeedReason::Watched);
        assert_eq!(SeedReason::of(&signal(None, false, 0.3, 0), "movie"), SeedReason::Started);
        assert_eq!(SeedReason::of(&signal(None, false, 2.0, 0), "series"), SeedReason::Watched, "dos episodios vistos");
    }

    #[test]
    fn better_rated_titles_rank_a_little_higher() {
        assert!(quality_factor(Some(8.5)) > quality_factor(Some(6.0)));
        assert!(quality_factor(Some(6.0)) > quality_factor(Some(4.0)));
        assert_eq!(quality_factor(Some(10.0)), 1.15);
        assert_eq!(quality_factor(Some(1.0)), 0.8);
    }

    #[test]
    fn one_seed_does_not_fill_the_whole_row() {
        let ranked = vec![(1.0, 1, "A".to_string()), (0.99, 2, "A".to_string()), (0.98, 3, "A".to_string()), (0.9, 4, "B".to_string())];
        let order: Vec<i32> = diversify(ranked).into_iter().map(|(_, id, _)| id).collect();
        assert_eq!(order, vec![1, 4, 2, 3], "la de B sube por delante de la segunda de A");
    }
}
