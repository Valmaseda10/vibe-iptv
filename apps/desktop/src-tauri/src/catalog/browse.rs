//! Portada y filas de Películas, Series e Inicio, como en Netflix (2026-10-22).
//!
//! Cada visita trae una semilla del cliente (`seed`): con ella se elige la portada, qué
//! títulos entran en cada fila y en qué orden van las filas de descubrimiento. Abrir la app
//! dos o tres veces el mismo día no enseña lo mismo; dentro de una visita (misma semilla)
//! todo es estable, y volver de una ficha no lo baraja.
//!
//! - Lo personal va arriba y en orden fijo: Top 10, recomendadas, «Porque viste X», Mi
//!   lista, tendencias y novedades.
//! - Las filas de descubrimiento (géneros, aclamadas, clásicos, estrenos...) se barajan,
//!   con más probabilidad de ir arriba los géneros que más gustan al perfil.
//! - Cada fila elige entre un grupo mayor de candidatas, con más peso a las mejores y a las
//!   de los géneros del perfil, y un título no se repite en dos filas de descubrimiento.
//! - Lo que el perfil ya conoce (visto, empezado, valorado o en su lista) no se le
//!   «descubre», y lo que marcó «No es para mí» no sale en ninguna fila.
//!
//! Aquí solo se decide qué va dónde (ids); `server::api` lo rellena con títulos y portadas
//! y quita lo que el perfil no puede ver.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::catalog::recommendations::{genre_affinity, genre_aversion, seed_weight};
use crate::db::reads::{self, BrowseIndexRow, TasteSignal};

/// El índice del catálogo cambia solo al sincronizar: basta con releerlo cada pocos minutos.
const INDEX_TTL: Duration = Duration::from_secs(600);

type IndexCache = HashMap<&'static str, (Instant, Arc<Vec<BrowseIndexRow>>)>;
static INDEX_CACHE: LazyLock<Mutex<IndexCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Índice del catálogo de un tipo (`movie` o `series`), en memoria unos minutos.
pub fn cached_index(conn: &Connection, kind: &'static str) -> crate::db::Result<Arc<Vec<BrowseIndexRow>>> {
    if let Ok(cache) = INDEX_CACHE.lock() {
        if let Some((at, index)) = cache.get(kind) {
            if at.elapsed() < INDEX_TTL {
                return Ok(index.clone());
            }
        }
    }
    let index = Arc::new(reads::browse_index(conn, kind)?);
    if let Ok(mut cache) = INDEX_CACHE.lock() {
        cache.insert(kind, (Instant::now(), index.clone()));
    }
    Ok(index)
}

/// Gustos de un perfil a partir de lo que ha hecho con cada título (ver
/// `catalog::recommendations::seed_weight`).
pub fn taste_from_signals(signals: &[TasteSignal], kind: &str, now: i64) -> Taste {
    let weighted: Vec<(f64, &[i64])> = signals.iter().map(|s| (seed_weight(s, kind, now), s.genres.as_slice())).collect();
    Taste {
        known: signals.iter().map(|s| s.id).collect(),
        seen: signals.iter().filter(|s| s.watched > 0.0 || s.rating.is_some()).map(|s| s.id).collect(),
        disliked: signals.iter().filter(|s| s.rating == Some(-1)).map(|s| s.id).collect(),
        affinity: genre_affinity(&weighted),
        aversion: genre_aversion(&weighted),
    }
}

/// Títulos por fila.
pub const ROW_SIZE: usize = 20;
/// Una fila con menos no se enseña.
pub const MIN_ROW: usize = 6;
/// Entre cuántas candidatas se elige cada fila de descubrimiento.
const POOL_SIZE: usize = 60;
/// Filas de género como mucho en una pantalla.
const MAX_GENRE_ROWS: usize = 8;

/// Generador pseudoaleatorio pequeño (splitmix64): reproducible con la misma semilla, sin
/// dependencias.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Entre 0 (incluido) y 1 (excluido).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Muestra ponderada sin repetición (Efraimidis–Spirakis): `k` de `items` al azar, con más
/// probabilidad (y antes en el resultado) cuanto más peso. Los de peso 0 o menos, nunca.
pub fn weighted_sample<T: Clone>(rng: &mut Rng, items: &[(T, f64)], k: usize) -> Vec<T> {
    let mut keyed: Vec<(f64, &T)> = items
        .iter()
        .filter(|(_, weight)| *weight > 0.0)
        .map(|(item, weight)| (rng.next_f64().max(1e-12).powf(1.0 / weight), item))
        .collect();
    keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    keyed.into_iter().take(k).map(|(_, item)| item.clone()).collect()
}

/// Gustos del perfil para un tipo (películas o series).
#[derive(Debug, Clone, Default)]
pub struct Taste {
    /// Visto, empezado, valorado o en su lista: no se le «descubre».
    pub known: HashSet<i64>,
    /// Visto, empezado o valorado (lo de su lista no): no va en la portada.
    pub seen: HashSet<i64>,
    /// «No es para mí»: no sale en ninguna fila.
    pub disliked: HashSet<i64>,
    /// Cuánto le gusta cada género, de 0 a 1.
    pub affinity: HashMap<i64, f64>,
    /// Cuánto le disgusta cada género, de 0 a 1.
    pub aversion: HashMap<i64, f64>,
}

/// Lo que hace falta para planificar la pantalla de un tipo.
pub struct KindInputs<'a> {
    /// `movie` o `series`.
    pub kind: &'static str,
    /// Catálogo emparejado con TMDB y con portada.
    pub index: &'a [BrowseIndexRow],
    pub taste: &'a Taste,
    pub genre_names: &'a HashMap<i64, String>,
    /// Top 10 en España, en orden.
    pub top: &'a [i64],
    /// Tendencias de la semana en TMDB, en orden.
    pub trending: &'a [i64],
    /// Recomendadas para el perfil, de más a menos, con la semilla que más aportó.
    pub recommended: &'a [(i64, String)],
    /// Filas «Porque viste X», de la semilla más fuerte a la más débil.
    pub because: &'a [(String, Vec<i64>)],
    /// Lo último que añadió el proveedor, de lo más nuevo a lo más viejo.
    pub recent: &'a [i64],
    /// Mi lista, de lo último añadido a lo primero.
    pub my_list: &'a [i64],
    pub now_year: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub kind: &'static str,
    pub id: i64,
    /// Por qué está ahí, si aporta algo («Porque te gustó X»).
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStyle {
    Poster,
    /// Con el número grande al lado (Top 10).
    Top10,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRow {
    /// Estable entre visitas («top», «genre-28»...): la interfaz la usa como clave.
    pub key: String,
    pub title: String,
    pub style: RowStyle,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Featured {
    pub kind: &'static str,
    pub id: i64,
    /// Lo que se enseña encima del título: «N.º 2 en España hoy», «Recomendada para ti»...
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    pub featured: Option<Featured>,
    pub rows: Vec<PlannedRow>,
}

/// Una fila de descubrimiento posible, antes de elegir sus títulos.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Discovery {
    Genre(i64),
    Acclaimed,
    Classics,
    Gems,
    Releases,
    /// La segunda fila «Porque viste X».
    SecondBecause,
}

/// Géneros de TMDB que no dan para una fila: telefilmes, noticias y programas de entrevistas.
const SKIPPED_GENRES: [i64; 3] = [10770, 10763, 10767];
/// Géneros que no cuentan como «aclamados» ni como portada: documentales, música, reality,
/// telefilmes, noticias, entrevistas (notas altas con muy pocos votos).
const NOT_ACCLAIMED_GENRES: [i64; 6] = [99, 10402, 10764, 10770, 10763, 10767];
/// Por encima de esto, en TMDB casi siempre es una nota con un puñado de votos.
const MAX_CREDIBLE_RATING: f64 = 9.3;

/// Nota que se puede creer: entre `min` y `MAX_CREDIBLE_RATING`, con fondo y sinopsis (lo
/// que tiene datos completos en TMDB suele tener votos de verdad) y fuera de los géneros
/// de nota poco fiable.
fn credible(entry: &BrowseIndexRow, min: f64) -> bool {
    entry.hero_ready && entry.rating.is_some_and(|r| r >= min && r <= MAX_CREDIBLE_RATING) && !entry.genres.iter().any(|g| NOT_ACCLAIMED_GENRES.contains(g))
}

/// Nombre de un género para una fila. TMDB da algunos de series sin traducir.
fn genre_label(kind: &str, genre: i64, name: &str, home: bool) -> String {
    let series = kind == "series";
    let translated = match (series, genre) {
        (true, 10759) => "Acción y aventura",
        (true, 10765) => "Ciencia ficción y fantasía",
        (true, 10762) => "Infantiles",
        (true, 10766) => "Telenovelas",
        (true, 10764) => "Realities",
        (true, 10768) => "Bélicas y de política",
        (_, 10751) => "Para ver en familia",
        (_, 99) => "Documentales",
        (false, 10402) => "Musicales",
        _ => name,
    };
    if !home {
        return translated.to_string();
    }
    match (series, genre) {
        (true, 10762) => "Series infantiles".into(),
        (true, 10766) => "Telenovelas".into(),
        (true, 10764) => "Realities".into(),
        (false, 10751) => "Películas para ver en familia".into(),
        (true, 10751) => "Series para ver en familia".into(),
        (_, 99) => "Documentales".into(),
        (false, 10402) => "Películas musicales".into(),
        (true, _) => format!("Series de {}", lower_first(translated)),
        (false, _) => format!("Películas de {}", lower_first(translated)),
    }
}

/// Peso de un título en una fila de descubrimiento: su nota, los géneros del perfil y si
/// es reciente.
fn discovery_weight(entry: &BrowseIndexRow, taste: &Taste, now_year: i32) -> f64 {
    let quality = entry.rating.filter(|r| *r <= MAX_CREDIBLE_RATING).map(|r| ((r - 4.0) / 5.0).clamp(0.1, 1.2)).unwrap_or(0.3);
    let affinity = entry.genres.iter().filter_map(|g| taste.affinity.get(g)).cloned().fold(0.0, f64::max);
    let aversion = entry.genres.iter().filter_map(|g| taste.aversion.get(g)).cloned().fold(0.0, f64::max);
    let fresh = if entry.year.is_some_and(|y| y >= now_year - 3) { 1.15 } else { 1.0 };
    quality * (1.0 + 0.8 * affinity) * (1.0 - 0.6 * aversion).max(0.2) * fresh
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Título de una fila de descubrimiento. En Inicio se dice si son películas o series.
fn discovery_title(kind: &str, discovery: Discovery, genre_names: &HashMap<i64, String>, taste: &Taste, home: bool, because_seed: Option<&str>) -> Option<String> {
    let series = kind == "series";
    Some(match discovery {
        Discovery::Genre(genre) => {
            let name = genre_names.get(&genre)?;
            let liked = taste.affinity.get(&genre).is_some_and(|a| *a >= 0.6);
            let label = genre_label(kind, genre, name, home);
            if liked && !home {
                format!("Más {} para ti", lower_first(&label))
            } else {
                label
            }
        }
        Discovery::Acclaimed => (if series { "Series aclamadas por la crítica" } else if home { "Películas aclamadas por la crítica" } else { "Aclamadas por la crítica" }).to_string(),
        Discovery::Classics => (if series { "Series de culto" } else { "Clásicos imprescindibles" }).to_string(),
        Discovery::Gems => (if series { "Series que quizá te perdiste" } else { "Joyas que quizá te perdiste" }).to_string(),
        Discovery::Releases => (if series { "Series recién estrenadas" } else { "Estrenos" }).to_string(),
        Discovery::SecondBecause => format!("Porque viste {}", because_seed?),
    })
}

fn discovery_matches(discovery: Discovery, entry: &BrowseIndexRow, series: bool, now_year: i32, prominent: &HashSet<i64>) -> bool {
    match discovery {
        Discovery::Genre(genre) => entry.genres.contains(&genre),
        Discovery::Acclaimed => credible(entry, 7.5),
        Discovery::Classics => entry.year.is_some_and(|y| y <= now_year - if series { 15 } else { 30 }) && credible(entry, 7.0),
        Discovery::Gems => entry.year.is_some_and(|y| y >= now_year - 15 && y <= now_year - 3) && credible(entry, 7.0) && !prominent.contains(&entry.id),
        Discovery::Releases => entry.year.is_some_and(|y| y >= now_year - 1),
        Discovery::SecondBecause => false,
    }
}

/// Elige los títulos de una fila de descubrimiento entre sus mejores candidatas.
fn pick_discovery(rng: &mut Rng, input: &KindInputs, discovery: Discovery, used: &HashSet<i64>, prominent: &HashSet<i64>) -> Vec<i64> {
    let series = input.kind == "series";
    let mut pool: Vec<(i64, f64)> = input
        .index
        .iter()
        .filter(|e| !input.taste.known.contains(&e.id) && !input.taste.disliked.contains(&e.id) && !used.contains(&e.id))
        .filter(|e| discovery_matches(discovery, e, series, input.now_year, prominent))
        .map(|e| (e.id, discovery_weight(e, input.taste, input.now_year)))
        .collect();
    pool.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    pool.truncate(POOL_SIZE);
    weighted_sample(rng, &pool, ROW_SIZE)
}

fn cards(kind: &'static str, ids: impl IntoIterator<Item = i64>) -> Vec<Card> {
    ids.into_iter().map(|id| Card { kind, id, reason: None }).collect()
}

/// Perfil sin gustos todavía (nada visto, valorado ni emparejado con TMDB): lo popular
/// (Top 10 y tendencias, alternados) y lo mejor valorado de los últimos años.
fn cold_start(input: &KindInputs) -> Vec<(i64, String)> {
    let fresh = |id: &i64| !input.taste.known.contains(id) && !input.taste.disliked.contains(id);
    let mut ids: Vec<i64> = Vec::new();
    let (mut top, mut trending) = (input.top.iter().filter(|id| fresh(*id)), input.trending.iter().filter(|id| fresh(*id)));
    loop {
        let (a, b) = (top.next(), trending.next());
        if a.is_none() && b.is_none() {
            break;
        }
        ids.extend(a.into_iter().chain(b).copied());
    }
    let mut acclaimed: Vec<&BrowseIndexRow> = input.index.iter().filter(|e| fresh(&e.id) && e.year.is_some_and(|y| y >= input.now_year - 5) && credible(e, 7.0)).collect();
    acclaimed.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(std::cmp::Ordering::Equal).then(a.id.cmp(&b.id)));
    ids.extend(acclaimed.into_iter().take(30).map(|e| e.id));
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).take(40).map(|id| (id, String::new())).collect()
}

/// Recomendadas: siempre de las mejores, pero no las mismas ni en el mismo orden cada vez.
/// Sin gustos todavía, lo popular (ver [`cold_start`]).
fn pick_recommended(rng: &mut Rng, input: &KindInputs, take: usize) -> Vec<Card> {
    let source = if input.recommended.is_empty() { cold_start(input) } else { input.recommended.to_vec() };
    let pool: Vec<((i64, String), f64)> = source
        .into_iter()
        .filter(|(id, _)| !input.taste.disliked.contains(id))
        .enumerate()
        .map(|(rank, item)| (item, 1.0 / (1.0 + 0.12 * rank as f64)))
        .collect();
    weighted_sample(rng, &pool, take)
        .into_iter()
        .map(|(id, because)| Card { kind: input.kind, id, reason: (!because.is_empty()).then(|| format!("Porque te gustó {because}")) })
        .collect()
}

/// Candidatas a la portada, con su peso y el porqué. Solo las que tienen fondo y sinopsis.
fn featured_candidates(input: &KindInputs) -> Vec<(Featured, f64)> {
    let ready: HashMap<i64, &BrowseIndexRow> = input.index.iter().filter(|e| e.hero_ready).map(|e| (e.id, e)).collect();
    // Lo de su lista sí puede salir: añadir la portada a «Mi lista» no la cambia.
    let eligible = |id: &i64| ready.contains_key(id) && !input.taste.seen.contains(id) && !input.taste.disliked.contains(id);
    let mut by_id: HashMap<i64, (Featured, f64)> = HashMap::new();
    let mut offer = |id: i64, weight: f64, reason: String| {
        let entry = by_id.entry(id).or_insert_with(|| (Featured { kind: input.kind, id, reason: reason.clone() }, 0.0));
        if weight > entry.1 {
            *entry = (Featured { kind: input.kind, id, reason }, weight);
        }
    };
    for (rank, (id, _)) in input.recommended.iter().filter(|(id, _)| eligible(id)).take(10).enumerate() {
        offer(*id, 3.0 / (1.0 + 0.2 * rank as f64), "Recomendada para ti".to_string());
    }
    for (rank, id) in input.top.iter().enumerate().filter(|(_, id)| eligible(*id)) {
        offer(*id, 2.0 / (1.0 + 0.1 * rank as f64), format!("N.º {} en España hoy", rank + 1));
    }
    for (rank, id) in input.trending.iter().filter(|id| eligible(*id)).take(10).enumerate() {
        offer(*id, 1.5 / (1.0 + 0.1 * rank as f64), "Tendencia esta semana".to_string());
    }
    let mut releases: Vec<&BrowseIndexRow> = ready
        .values()
        .filter(|e| eligible(&e.id) && e.year.is_some_and(|y| y >= input.now_year - 2) && credible(e, 7.0))
        .copied()
        .collect();
    releases.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(std::cmp::Ordering::Equal).then(a.id.cmp(&b.id)));
    for entry in releases.into_iter().take(8) {
        offer(entry.id, 0.5, "Estreno aclamado".to_string());
    }
    let mut candidates: Vec<(Featured, f64)> = by_id.into_values().collect();
    candidates.sort_by_key(|(f, _)| f.id);
    candidates
}

/// Elige la portada. Evita las de las últimas visitas (`avoid`) mientras haya otras.
fn pick_featured(rng: &mut Rng, candidates: Vec<(Featured, f64)>, avoid: &HashSet<(&'static str, i64)>) -> Option<Featured> {
    let fresh: Vec<(Featured, f64)> = candidates.iter().filter(|(f, _)| !avoid.contains(&(f.kind, f.id))).cloned().collect();
    let pool = if fresh.is_empty() { candidates } else { fresh };
    weighted_sample(rng, &pool, 1).into_iter().next()
}

/// Pantalla de Películas o de Series. `home` cambia los títulos de las filas de
/// descubrimiento para Inicio («Películas de acción» en vez de «Acción»).
pub fn plan_kind(input: &KindInputs, seed: u64, avoid: &HashSet<(&'static str, i64)>, home: bool) -> Plan {
    let mut rng = Rng::new(seed ^ if input.kind == "series" { 0x5E1E5 } else { 0x0F11A });
    let series = input.kind == "series";
    let mut rows = Vec::new();
    let mut used: HashSet<i64> = HashSet::new();
    let push = |rows: &mut Vec<PlannedRow>, key: &str, title: String, style: RowStyle, cards: Vec<Card>, min: usize| {
        if cards.len() >= min {
            rows.push(PlannedRow { key: key.to_string(), title, style, cards });
        }
    };

    let featured = pick_featured(&mut rng, featured_candidates(input), avoid);
    if let Some(f) = &featured {
        used.insert(f.id);
    }

    let top: Vec<i64> = input.top.iter().copied().filter(|id| !input.taste.disliked.contains(id)).take(10).collect();
    used.extend(&top);
    let top_title = if series { "Top 10 series en España hoy" } else { "Top 10 películas en España hoy" };
    push(&mut rows, "top", top_title.to_string(), RowStyle::Top10, cards(input.kind, top), 3);

    let recommended = pick_recommended(&mut rng, input, ROW_SIZE);
    used.extend(recommended.iter().map(|c| c.id));
    let recommended_title = match (series, input.recommended.is_empty()) {
        (true, false) => "Series recomendadas para ti",
        (false, false) => "Recomendadas para ti",
        (true, true) => "Series populares ahora",
        (false, true) => "Populares ahora",
    };
    push(&mut rows, "recommended", recommended_title.to_string(), RowStyle::Poster, recommended, MIN_ROW);

    // Las filas «Porque viste X» que salen esta vez: una arriba y quizá otra más abajo.
    let because_pool: Vec<(usize, f64)> = (0..input.because.len()).map(|i| (i, 1.0 / (1.0 + i as f64))).collect();
    let because_order = weighted_sample(&mut rng, &because_pool, 2);
    let because_row = |index: usize, used: &HashSet<i64>| -> Option<(String, Vec<i64>)> {
        let (seed_title, ids) = input.because.get(index)?;
        let clean: Vec<i64> = ids.iter().copied().filter(|id| !input.taste.disliked.contains(id) && !input.taste.known.contains(id)).collect();
        let fresh: Vec<i64> = clean.iter().copied().filter(|id| !used.contains(id)).collect();
        Some((seed_title.clone(), if fresh.len() >= MIN_ROW { fresh } else { clean }))
    };
    if let Some((seed_title, ids)) = because_order.first().and_then(|i| because_row(*i, &used)) {
        let ids: Vec<i64> = ids.into_iter().take(ROW_SIZE).collect();
        used.extend(&ids);
        push(&mut rows, "because-1", format!("Porque viste {seed_title}"), RowStyle::Poster, cards(input.kind, ids), 4);
    }

    push(&mut rows, "my-list", "Mi lista".to_string(), RowStyle::Poster, cards(input.kind, input.my_list.iter().copied().take(40)), 1);

    let trending: Vec<i64> = input.trending.iter().copied().filter(|id| !input.taste.disliked.contains(id)).take(ROW_SIZE).collect();
    used.extend(&trending);
    let trending_title = if series { "Series en tendencia" } else { "Tendencias esta semana" };
    push(&mut rows, "trending", trending_title.to_string(), RowStyle::Poster, cards(input.kind, trending), MIN_ROW);

    let recent: Vec<i64> = input
        .recent
        .iter()
        .copied()
        .filter(|id| !input.taste.disliked.contains(id) && !input.taste.known.contains(id))
        .take(ROW_SIZE)
        .collect();
    push(&mut rows, "new", "Novedades".to_string(), RowStyle::Poster, cards(input.kind, recent), MIN_ROW);

    // Descubrimiento: géneros con títulos de sobra y unas cuantas filas temáticas,
    // barajadas con más peso a lo que gusta.
    let prominent: HashSet<i64> = input.top.iter().chain(input.trending.iter()).copied().collect();
    let mut genre_counts: HashMap<i64, usize> = HashMap::new();
    for entry in input.index {
        for genre in &entry.genres {
            *genre_counts.entry(*genre).or_default() += 1;
        }
    }
    let mut slots: Vec<(Discovery, f64)> = genre_counts
        .iter()
        .filter(|(genre, count)| **count >= MIN_ROW * 2 && input.genre_names.contains_key(genre) && !SKIPPED_GENRES.contains(genre))
        .map(|(genre, count)| {
            let affinity = input.taste.affinity.get(genre).copied().unwrap_or(0.0);
            let aversion = input.taste.aversion.get(genre).copied().unwrap_or(0.0);
            (Discovery::Genre(*genre), (*count as f64).powf(0.25) * (1.0 + 3.0 * affinity) * (1.0 - 0.7 * aversion).max(0.1))
        })
        .collect();
    slots.sort_by_key(|(d, _)| match d {
        Discovery::Genre(g) => *g,
        _ => 0,
    });
    let base = slots.iter().map(|(_, w)| *w).fold(0.0, f64::max).max(1.0) * 0.6;
    slots.extend([(Discovery::Acclaimed, base), (Discovery::Classics, base * 0.8), (Discovery::Gems, base * 0.8), (Discovery::Releases, base)]);
    if because_order.len() > 1 {
        slots.push((Discovery::SecondBecause, base * 1.5));
    }
    let order = weighted_sample(&mut rng, &slots, slots.len());
    let mut genre_rows = 0;
    for discovery in order {
        if let Discovery::Genre(_) = discovery {
            if genre_rows >= MAX_GENRE_ROWS {
                continue;
            }
        }
        let (key, title, ids) = match discovery {
            Discovery::SecondBecause => {
                let Some((seed_title, ids)) = because_order.get(1).and_then(|i| because_row(*i, &used)) else { continue };
                ("because-2".to_string(), discovery_title(input.kind, discovery, input.genre_names, input.taste, home, Some(&seed_title)), ids.into_iter().take(ROW_SIZE).collect())
            }
            _ => {
                let key = match discovery {
                    Discovery::Genre(g) => format!("genre-{g}"),
                    Discovery::Acclaimed => "acclaimed".into(),
                    Discovery::Classics => "classics".into(),
                    Discovery::Gems => "gems".into(),
                    _ => "releases".into(),
                };
                (key, discovery_title(input.kind, discovery, input.genre_names, input.taste, home, None), pick_discovery(&mut rng, input, discovery, &used, &prominent))
            }
        };
        let Some(title) = title else { continue };
        if ids.len() < MIN_ROW {
            continue;
        }
        if let Discovery::Genre(_) = discovery {
            genre_rows += 1;
        }
        used.extend(&ids);
        rows.push(PlannedRow { key, title, style: RowStyle::Poster, cards: cards(input.kind, ids) });
    }

    Plan { featured, rows }
}

/// Mezcla dos listas alternando (película, serie, película...), sin pasar de `take`.
fn interleave(a: Vec<Card>, b: Vec<Card>, take: usize) -> Vec<Card> {
    let mut out = Vec::new();
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    loop {
        let (x, y) = (a.next(), b.next());
        if x.is_none() && y.is_none() {
            break;
        }
        out.extend(x);
        out.extend(y);
    }
    out.truncate(take);
    out
}

/// Inicio: una portada de cualquiera de los dos tipos y una mezcla corta de lo mejor de
/// cada uno (recomendado, «Porque viste X», tendencias, novedades y un par de filas de
/// descubrimiento). Continuar viendo, Mi lista y los canales los pone la interfaz.
pub fn plan_home(movies: &KindInputs, series: &KindInputs, seed: u64, avoid: &HashSet<(&'static str, i64)>) -> Plan {
    let mut rng = Rng::new(seed ^ 0x40DE);
    let mut candidates = featured_candidates(movies);
    candidates.extend(featured_candidates(series));
    let featured = pick_featured(&mut rng, candidates, avoid);

    let movie_plan = plan_kind(movies, seed, avoid, true);
    let series_plan = plan_kind(series, seed, avoid, true);
    let row = |plan: &Plan, key: &str| plan.rows.iter().find(|r| r.key == key).map(|r| r.cards.clone()).unwrap_or_default();
    let not_featured = |cards: Vec<Card>| -> Vec<Card> { cards.into_iter().filter(|c| featured.as_ref().is_none_or(|f| !(f.kind == c.kind && f.id == c.id))).collect() };

    let mut rows = Vec::new();
    let recommended = interleave(not_featured(pick_recommended(&mut rng, movies, 12)), not_featured(pick_recommended(&mut rng, series, 10)), ROW_SIZE);
    if recommended.len() >= MIN_ROW {
        let personal = !movies.recommended.is_empty() || !series.recommended.is_empty();
        let title = if personal { "Recomendado para ti" } else { "Populares ahora" };
        rows.push(PlannedRow { key: "recommended".into(), title: title.into(), style: RowStyle::Poster, cards: recommended });
    }
    // Una sola «Porque viste X», de películas o de series.
    let because: Vec<&PlannedRow> = movie_plan.rows.iter().chain(series_plan.rows.iter()).filter(|r| r.key == "because-1").collect();
    let because_pool: Vec<(usize, f64)> = (0..because.len()).map(|i| (i, 1.0)).collect();
    if let Some(index) = weighted_sample(&mut rng, &because_pool, 1).first() {
        let mut row = because[*index].clone();
        row.cards = not_featured(row.cards);
        if row.cards.len() >= 4 {
            rows.push(row);
        }
    }
    let trending = interleave(not_featured(row(&movie_plan, "trending")), not_featured(row(&series_plan, "trending")), ROW_SIZE);
    if trending.len() >= MIN_ROW {
        rows.push(PlannedRow { key: "trending".into(), title: "Tendencias ahora".into(), style: RowStyle::Poster, cards: trending });
    }
    let recent = interleave(row(&movie_plan, "new"), row(&series_plan, "new"), ROW_SIZE);
    if recent.len() >= MIN_ROW {
        rows.push(PlannedRow { key: "new".into(), title: "Novedades".into(), style: RowStyle::Poster, cards: recent });
    }
    // La primera fila de descubrimiento de cada tipo (ya barajadas con la semilla).
    let is_discovery = |r: &&PlannedRow| r.key.starts_with("genre-") || matches!(r.key.as_str(), "acclaimed" | "classics" | "gems" | "releases");
    for plan in [&movie_plan, &series_plan] {
        if let Some(found) = plan.rows.iter().find(is_discovery) {
            let mut found = found.clone();
            found.key = format!("{}-{}", found.cards.first().map(|c| c.kind).unwrap_or("x"), found.key);
            found.cards = not_featured(found.cards);
            if found.cards.len() >= MIN_ROW {
                rows.push(found);
            }
        }
    }
    Plan { featured, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, rating: f64, year: i32, genres: &[i64]) -> BrowseIndexRow {
        BrowseIndexRow { id, rating: Some(rating), year: Some(year), genres: genres.to_vec(), hero_ready: true }
    }

    /// 200 títulos: géneros 28 (acción), 35 (comedia) y 18 (drama), notas y años variados.
    fn catalog() -> Vec<BrowseIndexRow> {
        (1..=200)
            .map(|id| {
                let genre = [28, 35, 18][(id % 3) as usize];
                entry(id, 5.0 + (id % 40) as f64 / 10.0, 1960 + (id % 66) as i32, &[genre])
            })
            .collect()
    }

    fn names() -> HashMap<i64, String> {
        [(28, "Acción"), (35, "Comedia"), (18, "Drama")].into_iter().map(|(id, n)| (id, n.to_string())).collect()
    }

    fn inputs<'a>(index: &'a [BrowseIndexRow], taste: &'a Taste, names: &'a HashMap<i64, String>, recommended: &'a [(i64, String)]) -> KindInputs<'a> {
        KindInputs {
            kind: "movie",
            index,
            taste,
            genre_names: names,
            top: &[1, 2, 3, 4, 5],
            trending: &[10, 11, 12, 13, 14, 15, 16],
            recommended,
            because: &[],
            recent: &[200, 199, 198, 197, 196, 195, 194],
            my_list: &[],
            now_year: 2026,
        }
    }

    fn all_ids(plan: &Plan) -> Vec<Vec<i64>> {
        plan.rows.iter().map(|r| r.cards.iter().map(|c| c.id).collect()).collect()
    }

    #[test]
    fn the_same_seed_gives_the_same_screen_and_another_seed_a_different_one() {
        let index = catalog();
        let (taste, names) = (Taste::default(), names());
        let recommended: Vec<(i64, String)> = (50..90).map(|id| (id, "Matrix".to_string())).collect();
        let input = inputs(&index, &taste, &names, &recommended);
        let avoid = HashSet::new();
        let a = plan_kind(&input, 1, &avoid, false);
        assert_eq!(a, plan_kind(&input, 1, &avoid, false), "misma visita, misma pantalla");
        let differs = (2..6).any(|seed| {
            let b = plan_kind(&input, seed, &avoid, false);
            all_ids(&a) != all_ids(&b) || a.featured != b.featured
        });
        assert!(differs, "otra visita cambia filas o portada");
    }

    #[test]
    fn personal_rows_come_first_in_a_fixed_order() {
        let index = catalog();
        let (taste, names) = (Taste::default(), names());
        let recommended: Vec<(i64, String)> = (50..90).map(|id| (id, "Matrix".to_string())).collect();
        let plan = plan_kind(&inputs(&index, &taste, &names, &recommended), 7, &HashSet::new(), false);
        let keys: Vec<&str> = plan.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(&keys[..4], ["top", "recommended", "trending", "new"]);
        assert_eq!(plan.rows[0].style, RowStyle::Top10);
        assert!(plan.rows[1].cards.iter().all(|c| c.reason.as_deref() == Some("Porque te gustó Matrix")));
    }

    #[test]
    fn discovery_rows_never_repeat_a_title_nor_show_known_or_disliked_ones() {
        let index = catalog();
        let mut taste = Taste::default();
        taste.known.extend(100..140);
        taste.disliked.extend([141, 142, 143]);
        let names = names();
        let plan = plan_kind(&inputs(&index, &taste, &names, &[]), 3, &HashSet::new(), false);
        let mut seen = HashSet::new();
        for row in plan.rows.iter().filter(|r| r.key.starts_with("genre-") || ["acclaimed", "classics", "gems", "releases"].contains(&r.key.as_str())) {
            for card in &row.cards {
                assert!(seen.insert(card.id), "{} repetida en {}", card.id, row.key);
                assert!(!taste.known.contains(&card.id) && !taste.disliked.contains(&card.id));
            }
        }
        assert!(seen.len() > 30, "hay filas de descubrimiento");
    }

    #[test]
    fn favourite_genres_tend_to_come_first_and_get_a_personal_title() {
        let index = catalog();
        let mut taste = Taste::default();
        taste.affinity.insert(18, 1.0);
        let names = names();
        let input = inputs(&index, &taste, &names, &[]);
        let mut drama_first = 0;
        for seed in 0..40 {
            let plan = plan_kind(&input, seed, &HashSet::new(), false);
            let first_genre = plan.rows.iter().find(|r| r.key.starts_with("genre-")).unwrap();
            if first_genre.key == "genre-18" {
                drama_first += 1;
                assert_eq!(first_genre.title, "Más drama para ti");
            }
        }
        assert!(drama_first > 20, "el género que más gusta sale primero casi siempre ({drama_first}/40)");
    }

    #[test]
    fn the_featured_title_avoids_the_last_ones_shown() {
        let index = catalog();
        let (taste, names) = (Taste::default(), names());
        let recommended: Vec<(i64, String)> = vec![(60, "X".into()), (61, "X".into())];
        let input = KindInputs { top: &[], trending: &[], recent: &[], ..inputs(&index, &taste, &names, &recommended) };
        // Solo dos candidatas fuertes, más los estrenos aclamados; si se evita una, nunca sale.
        let avoid: HashSet<(&'static str, i64)> = [("movie", 60)].into_iter().collect();
        for seed in 0..20 {
            let featured = plan_kind(&input, seed, &avoid, false).featured.unwrap();
            assert_ne!(featured.id, 60);
        }
    }

    #[test]
    fn home_mixes_movies_and_series() {
        let index = catalog();
        let (taste, names) = (Taste::default(), names());
        let rec_movies: Vec<(i64, String)> = (50..70).map(|id| (id, "A".to_string())).collect();
        let rec_series: Vec<(i64, String)> = (70..90).map(|id| (id, "B".to_string())).collect();
        let movies = inputs(&index, &taste, &names, &rec_movies);
        let series = KindInputs { kind: "series", ..inputs(&index, &taste, &names, &rec_series) };
        let plan = plan_home(&movies, &series, 9, &HashSet::new());
        let recommended = plan.rows.iter().find(|r| r.key == "recommended").unwrap();
        let kinds: Vec<&str> = recommended.cards.iter().take(4).map(|c| c.kind).collect();
        assert_eq!(kinds, ["movie", "series", "movie", "series"]);
        assert!(plan.rows.iter().any(|r| r.title.starts_with("Películas de ") || r.title.starts_with("Series de ") || r.title.contains("aclamadas") || r.title == "Estrenos" || r.title.starts_with("Clásicos") || r.title.starts_with("Joyas") || r.title.starts_with("Series ")));
    }

    #[test]
    fn a_profile_without_tastes_gets_popular_titles_instead_of_an_empty_row() {
        let index = catalog();
        let (taste, names) = (Taste::default(), names());
        let plan = plan_kind(&inputs(&index, &taste, &names, &[]), 4, &HashSet::new(), false);
        let row = plan.rows.iter().find(|r| r.key == "recommended").expect("fila de populares");
        assert_eq!(row.title, "Populares ahora");
        assert!(row.cards.iter().all(|c| c.reason.is_none()), "sin «Porque te gustó» inventado");
        assert!(row.cards.iter().filter(|c| [1, 2, 3, 4, 5, 10, 11, 12].contains(&c.id)).count() >= 5, "lo popular manda");
    }

    #[test]
    fn weighted_sampling_prefers_heavier_items() {
        let mut rng = Rng::new(42);
        let items = vec![("pesado", 10.0), ("ligero", 0.5), ("nulo", 0.0)];
        let mut heavy_first = 0;
        for _ in 0..200 {
            let picked = weighted_sample(&mut rng, &items, 2);
            assert!(!picked.contains(&"nulo"));
            if picked[0] == "pesado" {
                heavy_first += 1;
            }
        }
        assert!(heavy_first > 170);
    }
}
