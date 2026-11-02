//! Deportes (F17, parte 3): los partidos y carreras de la guía ya guardada, sin pedir nada
//! al proveedor.
//!
//! La guía no trae categorías en `<category>` (en la del usuario, ninguna de 10.688), pero
//! la de estilo Movistar Plus dice el deporte en la primera línea de la descripción
//! («Deportes,Fútbol | 2026 | TP») y marca el directo en el título («DIRECTO Jornada 11:
//! Getafe - Sevilla T26/27 · LALIGA EA SPORTS»). Lo demás sale del título: los dos rivales
//! («A - B», «A vs. B»), la competición (el otro trozo separado por « · ») y si es una
//! repetición (temporada que no es la actual), un resumen, una previa o un programa.
//!
//! Un partido que se emite en varios canales es un solo evento con todos sus canales. Cada
//! perfil sigue equipos y competiciones (`sport_follows`); con eso y lo grande que es el
//! partido (Madrid, Barça, Champions...) se decide cuál sale en la portada de Inicio.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{params, Connection};

use super::live_sports::{same_team, EspnTeam};
use crate::db::{now_epoch, reads, Result};

/// Lo que tiene que valer un partido para salir en la portada.
pub const FEATURE_MIN_SCORE: i64 = 70;
/// Lo que suma seguir a un equipo o una competición.
const FOLLOWED_SCORE: i64 = 100;

/// Equipos que se reconocen con varios nombres: clave estable, nombre bonito, alias (ya
/// normalizados: minúsculas, sin tildes ni puntos) e importancia para la portada.
const TEAMS: &[(&str, &str, &[&str], i64)] = &[
    ("real-madrid", "Real Madrid", &["real madrid", "r madrid", "real madrid cf", "madrid"], 40),
    ("barcelona", "FC Barcelona", &["barcelona", "fc barcelona", "barca", "fcb"], 40),
    ("atletico", "Atlético de Madrid", &["atletico de madrid", "atletico madrid", "atletico", "at madrid", "atl madrid", "atl de madrid"], 25),
    ("athletic", "Athletic Club", &["athletic", "athletic club", "athletic bilbao", "ath bilbao"], 10),
    ("real-sociedad", "Real Sociedad", &["real sociedad"], 10),
    ("betis", "Real Betis", &["betis", "real betis"], 10),
    ("sevilla", "Sevilla FC", &["sevilla", "sevilla fc"], 10),
    ("valencia", "Valencia CF", &["valencia", "valencia cf"], 10),
    ("villarreal", "Villarreal CF", &["villarreal", "villarreal cf"], 10),
    ("espana", "España", &["espana", "seleccion espanola"], 40),
];

/// Competiciones que se reconocen: clave, nombre, palabras que las delatan (normalizadas),
/// deporte e importancia.
const COMPETITIONS: &[(&str, &str, &[&str], &str, i64)] = &[
    ("champions", "Champions League", &["champions", "liga de campeones", "ligacampeones"], "futbol", 40),
    ("europa-league", "Europa League", &["europa league"], "futbol", 15),
    // Antes que «LaLiga»: «LALIGA HyperMotion» también contiene «laliga».
    ("laliga2", "LaLiga Hypermotion", &["hypermotion", "segunda division"], "futbol", 5),
    ("laliga", "LaLiga", &["laliga ea sports", "liga ea sports", "primera division", "laliga"], "futbol", 30),
    ("primera-federacion", "Primera Federación", &["primera federacion", "primera rfef", "1a rfef", "1 rfef"], "futbol", 3),
    ("copa-del-rey", "Copa del Rey", &["copa del rey"], "futbol", 25),
    ("premier", "Premier League", &["premier league"], "futbol", 20),
    ("serie-a", "Serie A", &["serie a"], "futbol", 10),
    ("bundesliga", "Bundesliga", &["bundesliga"], "futbol", 10),
    ("ligue-1", "Ligue 1", &["ligue 1"], "futbol", 5),
    ("mundial", "Mundial", &["copa mundial", "mundial de futbol", "copa del mundo de la fifa", "fifa world cup"], "futbol", 50),
    ("eurocopa", "Eurocopa", &["eurocopa"], "futbol", 50),
    ("nba", "NBA", &["nba"], "baloncesto", 15),
    ("euroliga", "Euroliga", &["euroliga", "euroleague"], "baloncesto", 15),
    ("acb", "Liga Endesa", &["liga endesa", "acb"], "baloncesto", 10),
    ("f1", "Fórmula 1", &["formula 1", "f1"], "motor", 25),
    ("motogp", "MotoGP", &["motogp"], "motor", 20),
    ("nfl", "NFL", &["nfl"], "futbol-americano", 5),
    ("mlb", "MLB", &["mlb"], "beisbol", 0),
];

/// Deportes por la primera línea de la descripción («Deportes,Fútbol»).
const SPORTS: &[(&str, &str)] = &[
    ("futbol sala", "futbol-sala"),
    ("futbol americano", "futbol-americano"),
    ("futbol", "futbol"),
    ("baloncesto", "baloncesto"),
    ("tenis", "tenis"),
    ("motor", "motor"),
    ("formula 1", "motor"),
    ("motociclismo", "motor"),
    ("balonmano", "balonmano"),
    ("ciclismo", "ciclismo"),
    ("golf", "golf"),
    ("boxeo", "combate"),
    ("artes marciales", "combate"),
    ("rugby", "rugby"),
    ("beisbol", "beisbol"),
];

/// Palabras de un programa que no es un evento: resúmenes, previas, tertulias...
const NOT_EVENT_WORDS: &[&str] = &[
    "resumen", "resumenes", "previa", "post", "magazine", "highlights", "clasicos", "programa", "estudio", "repeticion", "diferido",
    "features", "retro", "debate", "tertulia", "noticias", "sportscenter", "documental", "entrevista", "reportaje", "trending",
];

/// Lo que no puede ser un rival: «Eslalon gigante - 1ª manga», «Día 1 - Primera ronda».
const NOT_PARTICIPANT_WORDS: &[&str] = &[
    "manga", "ronda", "dia", "etapa", "jornada", "final", "semifinal", "semifinales", "cuartos", "octavos", "episodio", "capitulo", "parte",
    "programa", "masculino", "femenino", "clasificacion", "entrenamientos", "carrera", "finales", "eliminatoria", "clasificatoria",
    "velocidad", "bloque", "dificultad", "sprint", "prueba",
];

static SEASON: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\bT(\d{2})/(\d{2})\b").ok());
static SEASON_YEAR: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\bT(20\d{2})\b").ok());
static SEASON_TAIL: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\s+T(\d{2}/\d{2}|20\d{2})\b.*$").ok());
static VERSUS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\s+(?:-|–|vs\.?|v\.?)\s+").ok());

/// Minúsculas, sin tildes, sin puntos ni comillas y con un espacio entre palabras.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        let mapped = match ch {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            '.' | '\'' | '’' | '"' => continue,
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        out.push(mapped);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_word(normalized: &str, word: &str) -> bool {
    normalized.split(' ').any(|w| w == word)
}

/// Contiene la frase entera (por palabras).
fn has_phrase(normalized: &str, phrase: &str) -> bool {
    let haystack = format!(" {normalized} ");
    haystack.contains(&format!(" {phrase} "))
}

/// Temporada actual («26» de 26/27) y año, para reconocer repeticiones.
fn current_season(now: i64) -> (i64, i64) {
    let (year, month, _) = super::watch_time::civil_from_days(now.div_euclid(86_400));
    let start = if month >= 7 { year } else { year - 1 };
    (start % 100, year)
}

/// Un rival reconocido: su clave para seguirlo y el nombre que se enseña.
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    pub key: String,
    pub name: String,
}

pub fn participant(name: &str) -> Participant {
    let normalized = normalize(name);
    for (key, label, aliases, _) in TEAMS {
        if aliases.contains(&normalized.as_str()) {
            return Participant { key: (*key).to_string(), name: (*label).to_string() };
        }
    }
    Participant { key: normalized.replace(' ', "-"), name: name.trim().to_string() }
}

/// Si la clave es la de un equipo conocido (con sus alias), no la de un nombre suelto.
pub fn is_known_team(key: &str) -> bool {
    TEAMS.iter().any(|(k, ..)| *k == key)
}

fn team_weight(key: &str) -> i64 {
    TEAMS.iter().find(|(k, ..)| *k == key).map_or(0, |(.., weight)| *weight)
}

/// Lo que se saca de un programa que es un evento deportivo.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEvent {
    /// «Getafe - Sevilla» o, sin rivales, el título limpio («Final · Torneo de Viena»).
    pub name: String,
    pub participants: Vec<Participant>,
    pub competition: Option<String>,
    pub competition_key: Option<String>,
    pub sport: Option<String>,
    /// La guía dice que es en directo.
    pub live: bool,
}

fn strip_season(text: &str) -> String {
    match SEASON_TAIL.as_ref() {
        Some(re) => re.replace(text, "").trim().to_string(),
        None => text.trim().to_string(),
    }
}

/// Un nombre que la guía pone mientras no se sabe (TBC, TBD, «Por determinar»).
fn is_placeholder(text: &str) -> bool {
    matches!(normalize(text).as_str(), "tbc" | "tbd" | "tba" | "por determinar" | "por confirmar" | "a confirmar")
}

/// La ronda de una competición («Final», «Semifinales», «Jornada 3»), no su nombre.
fn is_round(text: &str) -> bool {
    let normalized = normalize(text);
    let first = normalized.split(' ').next().unwrap_or("");
    ["final", "semifinal", "semifinales", "cuartos", "octavos", "jornada", "ronda", "dia", "etapa", "manga", "grupo"].contains(&first)
}

/// Lo que delata una carrera o una prueba, no un rival: «GP de Malasia», «Mundial F1»,
/// «Copa del mundo de escalada» (visto en su guía el 2026-11-02).
const EVENT_NOT_TEAM: &[&str] = &["gp", "gran premio", "mundial", "copa del mundo", "motogp", "formula 1", "f1", "moto2", "moto3", "rally", "campeonato", "open", "torneo", "masters"];

fn valid_participant(side: &str) -> bool {
    let normalized = normalize(side);
    !EVENT_NOT_TEAM.iter().any(|w| has_phrase(&normalized, w))
        && !normalized.is_empty()
        && side.chars().count() <= 40
        && !normalized.starts_with(|c: char| c.is_ascii_digit())
        && normalized.split(' ').count() <= 5
        && !NOT_PARTICIPANT_WORDS.iter().any(|w| has_word(&normalized, w))
}

/// Los dos rivales de un trozo de título («Jornada 11: Rayo - Celta T26/27»).
fn matchup(segment: &str) -> Option<(String, String)> {
    let core = segment.rsplit_once(": ").map_or(segment, |(_, after)| after);
    let core = strip_season(core);
    let core = core.split(',').next().unwrap_or("").trim();
    let versus = VERSUS.as_ref()?;
    let mut parts = versus.splitn(core, 2);
    let (a, b) = (parts.next()?.trim(), parts.next()?.trim());
    if versus.is_match(b) || !valid_participant(a) || !valid_participant(b) {
        return None;
    }
    Some((a.to_string(), b.to_string()))
}

fn sport_from_description(description: Option<&str>) -> Option<String> {
    let first = normalize(description?.lines().next()?);
    let rest = first.strip_prefix("deportes ")?;
    SPORTS.iter().find(|(name, _)| rest.starts_with(name)).map(|(_, sport)| (*sport).to_string())
}

fn competition_of(text: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let normalized = normalize(text);
    COMPETITIONS
        .iter()
        .find(|(_, _, words, ..)| words.iter().any(|w| has_phrase(&normalized, w)))
        .map(|(key, name, _, sport, _)| (*key, *name, *sport))
}

/// Si un programa es un evento deportivo y cuál. `sporty_channel`: el canal es de deportes
/// (por su categoría o nombre); sin eso, solo cuenta si la descripción dice «Deportes».
pub fn parse_programme(title: &str, description: Option<&str>, sporty_channel: bool, now: i64) -> Option<ParsedEvent> {
    let mut text = title.trim();
    let mut live = false;
    for marker in ["DIRECTO ", "Directo: ", "EN DIRECTO: ", "En directo: "] {
        if let Some(rest) = text.strip_prefix(marker) {
            text = rest.trim();
            live = true;
        }
    }
    let normalized_title = normalize(text);
    if NOT_EVENT_WORDS.iter().any(|w| has_word(&normalized_title, w)) {
        return None;
    }
    let first_line = description.and_then(|d| d.lines().next()).map(normalize).unwrap_or_default();
    let described_sport = first_line.starts_with("deportes");
    if first_line.starts_with("deportes programa") || first_line.starts_with("documentales") || first_line.starts_with("entretenimiento") {
        return None;
    }
    if !sporty_channel && !described_sport {
        return None;
    }

    // Una temporada que no es la actual es una repetición.
    let (season, year) = current_season(now);
    if let Some(caps) = SEASON.as_ref().and_then(|re| re.captures(text)) {
        if caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) != Some(season) {
            return None;
        }
    } else if let Some(caps) = SEASON_YEAR.as_ref().and_then(|re| re.captures(text)) {
        if caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) != Some(year) {
            return None;
        }
    }

    // «TBC» (por confirmar) no dice nada: fuera.
    let segments: Vec<&str> = text.split(" · ").map(str::trim).filter(|s| !s.is_empty() && !is_placeholder(&strip_season(s))).collect();
    if segments.is_empty() {
        return None;
    }
    let found = segments.iter().enumerate().find_map(|(i, s)| matchup(s).map(|m| (i, m)));
    let competition_text = segments
        .iter()
        .enumerate()
        .filter(|(i, _)| found.as_ref().is_none_or(|(at, _)| at != i))
        .map(|(_, s)| strip_season(s))
        .find(|s| !s.is_empty() && s.chars().count() <= 60 && !is_round(s));
    let known = competition_text.as_deref().and_then(competition_of).or_else(|| competition_of(text));
    let sport = sport_from_description(description).or_else(|| known.map(|(.., sport)| sport.to_string()));

    let (name, participants) = match found {
        Some((_, (a, b))) => {
            let (a, b) = (participant(&a), participant(&b));
            (format!("{} - {}", a.name, b.name), vec![a, b])
        }
        // Sin rivales, solo si la guía dice que es en directo (una final de tenis, un gran
        // premio): si no, es imposible distinguirlo de una repetición.
        None if live && sport.is_some() => (segments.iter().map(|s| strip_season(s)).collect::<Vec<_>>().join(" · "), Vec::new()),
        // Una carrera (gran premio) es una carrera aunque la guía no diga «DIRECTO»: sin
        // rivales, con su nombre.
        None if sport.as_deref() == Some("motor") && (has_phrase(&normalized_title, "gp") || has_phrase(&normalized_title, "gran premio")) => {
            (segments.iter().map(|s| strip_season(s)).collect::<Vec<_>>().join(" · "), Vec::new())
        }
        None => return None,
    };
    let competition = known.map(|(_, name, _)| name.to_string()).or(competition_text);
    Some(ParsedEvent { name, participants, competition_key: known.map(|(key, ..)| key.to_string()), competition, sport, live })
}

/// Si un canal es de deportes, por su nombre o su categoría.
pub fn is_sporty_channel(channel_name: &str, category: Option<&str>) -> bool {
    const WORDS: &[&str] = &[
        "deporte", "deportes", "sport", "sports", "futbol", "dazn", "laliga", "liga", "champions", "campeones", "ligacampeones", "eurosport",
        "espn", "golf", "tenis", "tennis", "nba", "f1", "formula", "motor", "motogp", "ufc", "boxeo", "bein", "teledeporte", "gol", "tyc",
        "bundesliga", "premier",
    ];
    [Some(channel_name), category].into_iter().flatten().map(normalize).any(|n| WORDS.iter().any(|w| has_word(&n, w)) || n.contains("deport"))
}

/// Un canal en el que se ve un evento.
#[derive(Debug, Clone, PartialEq)]
pub struct EventChannel {
    pub id: i64,
    pub name: String,
    pub logo_url: Option<String>,
    pub quality: Option<String>,
}

/// Cuánto vale una calidad para elegir canal: 4K/UHD > FHD > HD > SD > sin marca.
pub fn quality_rank(quality: Option<&str>) -> i64 {
    match quality.map(|q| q.to_ascii_uppercase()) {
        Some(q) if q.contains("4K") || q.contains("UHD") => 4,
        Some(q) if q.contains("FHD") || q.contains("1080") => 3,
        Some(q) if q.contains("HD") || q.contains("720") => 2,
        Some(q) if q.contains("SD") => 1,
        _ => 0,
    }
}

/// Partido grande para la portada de Inicio cuando el perfil quiere «sus equipos y los
/// grandes» (decidido con el usuario): Madrid, Barça o Atlético en LaLiga, Copa o
/// Champions; la selección; y la Champions con cualquier equipo español.
pub fn is_big_match(event: &ParsedEvent) -> bool {
    let competition = event.competition_key.as_deref().unwrap_or("");
    let big = event.participants.iter().any(|p| matches!(p.key.as_str(), "real-madrid" | "barcelona" | "atletico" | "espana"));
    let spanish = event.participants.iter().any(|p| is_known_team(&p.key));
    (matches!(competition, "laliga" | "copa-del-rey" | "champions" | "mundial" | "eurocopa") && big) || (competition == "champions" && spanish)
}

/// Un evento con todos sus canales.
#[derive(Debug, Clone, PartialEq)]
pub struct SportEvent {
    pub key: String,
    pub event: ParsedEvent,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub channels: Vec<EventChannel>,
}

/// Lo que sigue un perfil.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Follows {
    pub teams: Vec<String>,
    /// Nombre de cada equipo seguido: la guía no siempre lo escribe igual que ESPN («Celta»
    /// y «Celta Vigo»), así que también se reconoce por el nombre.
    pub team_labels: Vec<String>,
    pub competitions: Vec<String>,
}

impl Follows {
    pub fn follows(&self, event: &ParsedEvent) -> bool {
        event.participants.iter().any(|p| self.teams.contains(&p.key) || self.team_labels.iter().any(|label| same_team(&p.name, &EspnTeam::named(label))))
            || event.competition_key.as_ref().is_some_and(|k| self.competitions.contains(k))
    }
}

/// Lo grande que es un evento para este perfil (la portada pide `FEATURE_MIN_SCORE`).
pub fn score(event: &ParsedEvent, follows: &Follows) -> i64 {
    let competition = event.competition_key.as_deref().and_then(|key| COMPETITIONS.iter().find(|(k, ..)| *k == key)).map_or(0, |(.., weight)| *weight);
    let teams: i64 = event.participants.iter().map(|p| team_weight(&p.key)).sum();
    let followed = if follows.follows(event) { FOLLOWED_SCORE } else { 0 };
    competition + teams + followed + if event.live { 10 } else { 0 }
}

fn event_key(event: &ParsedEvent, start_ts: i64) -> String {
    let mut who: Vec<&str> = event.participants.iter().map(|p| p.key.as_str()).collect();
    who.sort_unstable();
    let who = if who.is_empty() { normalize(&event.name).replace(' ', "-") } else { who.join("~") };
    // El mismo partido empieza a la misma hora en todos los canales (± media hora).
    format!("{who}@{}", (start_ts + 900) / 1800)
}

/// Los eventos de la guía entre `from` y `until` que el perfil puede ver, en orden de
/// hora, cada uno con todos sus canales.
pub fn events(conn: &Connection, from: i64, until: i64, access: reads::ProfileAccess) -> Result<Vec<SportEvent>> {
    let now = now_epoch();
    let mut stmt = conn.prepare(
        "SELECT p.title, p.description, p.start_ts, p.stop_ts, c.id, c.display_name, c.logo_url, cat.name, c.quality
         FROM epg_programmes p
         JOIN channels c ON c.epg_channel_id = p.channel_id AND (p.provider_id IS NULL OR p.provider_id = c.provider_id)
         LEFT JOIN categories cat ON cat.id = c.category_id
         WHERE p.start_ts < ?2 AND p.stop_ts > ?1
         ORDER BY p.start_ts, c.sort_order, c.id",
    )?;
    let rows = stmt.query_map(params![from, until], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    let mut allowed: HashMap<i64, bool> = HashMap::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<SportEvent> = Vec::new();
    for row in rows {
        let (title, description, start_ts, stop_ts, channel_id, channel_name, logo_url, category, quality) = row?;
        let sporty = is_sporty_channel(&channel_name, category.as_deref());
        let Some(parsed) = parse_programme(&title, description.as_deref(), sporty, now) else { continue };
        let ok = match allowed.get(&channel_id) {
            Some(ok) => *ok,
            None => {
                let ok = reads::item_allowed(conn, "channel", channel_id, access)?;
                allowed.insert(channel_id, ok);
                ok
            }
        };
        if !ok {
            continue;
        }
        let key = event_key(&parsed, start_ts);
        let channel = EventChannel { id: channel_id, name: channel_name, logo_url, quality };
        match by_key.get(&key) {
            Some(&index) => {
                let existing = &mut out[index];
                existing.event.live |= parsed.live;
                if existing.event.competition.is_none() {
                    existing.event.competition = parsed.competition;
                    existing.event.competition_key = parsed.competition_key;
                }
                // El mismo canal en otra calidad no es otro canal para elegir: se queda el
                // de mejor calidad.
                match existing.channels.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&channel.name)) {
                    Some(same) if quality_rank(channel.quality.as_deref()) > quality_rank(same.quality.as_deref()) => *same = channel,
                    Some(_) => {}
                    None => existing.channels.push(channel),
                }
            }
            None => {
                by_key.insert(key.clone(), out.len());
                out.push(SportEvent { key, event: parsed, start_ts, stop_ts, channels: vec![channel] });
            }
        }
    }
    Ok(out)
}

/// Lo que sigue un perfil.
pub fn follows(conn: &Connection, profile_id: i64) -> Result<Follows> {
    let mut stmt = conn.prepare("SELECT kind, key, label FROM sport_follows WHERE profile_id = ?1")?;
    let mut follows = Follows::default();
    for row in stmt.query_map(params![profile_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        let (kind, key, label) = row?;
        if kind == "team" {
            follows.teams.push(key);
            follows.team_labels.push(label);
        } else {
            follows.competitions.push(key);
        }
    }
    Ok(follows)
}

/// Algo que se puede seguir: un equipo (o jugador) o una competición.
#[derive(Debug, Clone, PartialEq)]
pub struct Followable {
    pub kind: &'static str,
    pub key: String,
    pub label: String,
    pub sport: Option<String>,
    /// Dónde sale en «Mis equipos»: «LaLiga», «LaLiga Hypermotion», «Primera Federación»...
    pub group: &'static str,
    pub logo: Option<String>,
}

/// Nombre de cada liga de ESPN en «Mis equipos».
fn league_group(league: &str) -> &'static str {
    match league {
        "soccer/esp.1" => "LaLiga",
        "soccer/esp.2" => "LaLiga Hypermotion",
        _ => "Otros equipos",
    }
}

/// Lo que se ofrece para seguir: los equipos de LaLiga y LaLiga Hypermotion (de ESPN, con
/// escudo, al día cada temporada), los conocidos, las competiciones y quien sale en la guía
/// de los próximos días (Primera Federación aparte; los que más salen, primero).
pub fn suggestions(upcoming: &[SportEvent], espn_teams: &[(&'static str, EspnTeam)]) -> Vec<Followable> {
    let espn_of = |label: &str| espn_teams.iter().find(|(_, t)| same_team(label, t));
    let mut out: Vec<Followable> = Vec::new();
    let already = |out: &[Followable], key: &str, label: &str| out.iter().any(|f| f.kind == "team" && (f.key == key || same_team(label, &EspnTeam::named(&f.label))));
    for (key, label, ..) in TEAMS {
        let espn = espn_of(label);
        let group = if *key == "espana" { "Selecciones" } else { espn.map_or("Otros equipos", |(league, _)| league_group(league)) };
        out.push(Followable { kind: "team", key: (*key).into(), label: (*label).into(), sport: Some("futbol".into()), group, logo: espn.and_then(|(_, t)| t.logo.clone()) });
    }
    for (league, team) in espn_teams {
        let key = participant(&team.name).key;
        if already(&out, &key, &team.name) {
            continue;
        }
        out.push(Followable { kind: "team", key, label: team.name.clone(), sport: Some("futbol".into()), group: league_group(league), logo: team.logo.clone() });
    }
    out.extend(
        COMPETITIONS
            .iter()
            .map(|(key, label, _, sport, _)| Followable { kind: "competition", key: (*key).into(), label: (*label).into(), sport: Some((*sport).into()), group: "Competiciones", logo: None }),
    );
    let mut seen: HashMap<String, (usize, Followable)> = HashMap::new();
    for event in upcoming {
        let group = if event.event.competition_key.as_deref() == Some("primera-federacion") { "Primera Federación" } else { "En tu guía" };
        for p in &event.event.participants {
            if already(&out, &p.key, &p.name) {
                continue;
            }
            let entry = seen.entry(p.key.clone()).or_insert_with(|| {
                (0, Followable { kind: "team", key: p.key.clone(), label: p.name.clone(), sport: event.event.sport.clone(), group, logo: None })
            });
            entry.0 += 1;
        }
    }
    let mut extra: Vec<(usize, Followable)> = seen.into_values().collect();
    extra.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    out.extend(extra.into_iter().map(|(_, f)| f));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-01: temporada 26/27.
    const NOW: i64 = 1_793_575_800;

    fn parse(title: &str, description: &str) -> Option<ParsedEvent> {
        parse_programme(title, Some(description), true, NOW)
    }

    #[test]
    fn live_league_match_with_competition_after_the_dot() {
        let event = parse("DIRECTO Jornada 11: Getafe - Sevilla T26/27 · LALIGA EA SPORTS", "Deportes,Fútbol | 2026 | TP\n· Los mejores clubes").unwrap();
        assert_eq!(event.name, "Getafe - Sevilla FC");
        assert!(event.live);
        assert_eq!(event.competition_key.as_deref(), Some("laliga"));
        assert_eq!(event.sport.as_deref(), Some("futbol"));
        assert_eq!(event.participants[1].key, "sevilla");
    }

    #[test]
    fn replays_summaries_and_shows_are_not_events() {
        assert!(parse("LALIGA EA SPORTS T25/26 · Jornada 22: Athletic - Real Sociedad", "Deportes,Fútbol | 2026 | TP").is_none(), "temporada pasada");
        assert!(parse("Resúmenes LALIGA HyperMotion T26/27 · Jornada 11: UD Almería - Real Oviedo", "Deportes,Fútbol | 2026 | TP").is_none());
        assert!(parse("DIRECTO Rayo - Celta T26/27 · DAZN Previa", "Deportes,Programa deportes | 2026 | TP").is_none());
        assert!(parse("Rayo - Celta T26/27 · DAZN Post", "Deportes,Programa deportes | 2026 | TP").is_none());
        assert!(parse("Magazine Champions League T26/27 E8", "Deportes,Programa deportes | 2026 | TP").is_none());
        assert!(parse("Boxeo: velada Ortiz Jr. vs Lubin T2025 · Vergil Ortiz Jr vs. Erikson Lubin", "Deportes,Boxeo | 2025 | +18").is_none(), "año pasado");
        assert!(parse("Copa del mundo de esquí alpino F - Sölden T26/27 · Eslalon gigante - 1ª manga", "Deportes,Esquí alpino | 2026 | TP").is_none(), "una manga no es un rival");
    }

    #[test]
    fn other_title_shapes() {
        let serie_a = parse_programme("Milan vs. Inter · Serie A", Some("· Sin detalles..."), true, NOW).unwrap();
        assert_eq!((serie_a.name.as_str(), serie_a.competition_key.as_deref()), ("Milan - Inter", Some("serie-a")));
        let mlb = parse_programme("MLB · Tampa Bay Rays vs. Milwaukee Brewers, Serie Mundial, American Family Field", None, true, NOW).unwrap();
        assert_eq!(mlb.name, "Tampa Bay Rays - Milwaukee Brewers");
        assert_eq!(mlb.competition.as_deref(), Some("MLB"));
        let tennis = parse("DIRECTO Semifinales: Medvedev - Alcaraz T2026 · Torneo de Viena", "Deportes,Tenis | 2026 | TP").unwrap();
        assert_eq!((tennis.name.as_str(), tennis.sport.as_deref(), tennis.competition.as_deref()), ("Medvedev - Alcaraz", Some("tenis"), Some("Torneo de Viena")));
        let cup = parse("Copa del Rey Mapfre T26/27 · Primera Ronda: El Prat - Athletic", "Deportes,Fútbol | 2026 | TP").unwrap();
        assert_eq!((cup.name.as_str(), cup.competition_key.as_deref()), ("El Prat - Athletic Club", Some("copa-del-rey")));
        let final_ = parse("DIRECTO Final T2026 · Torneo de Viena", "Deportes,Tenis | 2026 | TP").unwrap();
        assert_eq!(final_.name, "Final · Torneo de Viena");
        assert_eq!(final_.competition.as_deref(), Some("Torneo de Viena"), "la ronda no es la competición");
        let tbc = parse("DIRECTO TBC T2026 · Torneo de París", "Deportes,Tenis | 2026 | TP").unwrap();
        assert_eq!((tbc.name.as_str(), tbc.competition.as_deref()), ("Torneo de París", Some("Torneo de París")));
        let segunda = parse("DIRECTO LALIGA HYPERMOTION T26/27 · Jornada 13: Ceuta FC - Real Sporting", "Deportes,Fútbol | 2026 | TP").unwrap();
        assert_eq!(segunda.competition_key.as_deref(), Some("laliga2"), "Segunda no es LaLiga");
        assert!(parse("Final T2026 · Torneo de Viena", "Deportes,Tenis | 2026 | TP").is_none(), "sin rivales ni directo, puede ser una repetición");
        // Una carrera no tiene dos rivales.
        let gp = parse("MotoGP - GP de Malasia T2026", "Deportes,Motor | 2026 | TP").unwrap();
        assert!(gp.participants.is_empty(), "{gp:?}");
        assert_eq!(gp.name, "MotoGP - GP de Malasia");
        assert!(parse("Copa del mundo de escalada - Santiago de Chile T2026 · Velocidad - Finales", "Deportes,Escalada deportiva | 2026 | TP").is_none_or(|e| e.participants.is_empty()));
    }

    #[test]
    fn a_film_on_a_film_channel_is_not_a_match() {
        assert!(parse_programme("Kramer vs. Kramer", Some("Cine,Drama | 1979"), false, NOW).is_none());
        assert!(!is_sporty_channel("Cine Clásico", Some("CINE")));
        assert!(is_sporty_channel("DAZN LaLiga", Some("Primera Division por Dazn")));
        assert!(is_sporty_channel("M+ Liga de Campeones", Some("LIGACAMPEONES")));
    }

    #[test]
    fn big_matches_and_followed_teams_reach_the_front_page() {
        let none = Follows::default();
        let clasico = parse("DIRECTO Jornada 12: Real Madrid - Barcelona T26/27 · LALIGA EA SPORTS", "Deportes,Fútbol | 2026 | TP").unwrap();
        let small = parse("DIRECTO Jornada 11: Getafe - Sevilla T26/27 · LALIGA EA SPORTS", "Deportes,Fútbol | 2026 | TP").unwrap();
        let madrid = parse("DIRECTO Jornada 12: Real Madrid - Getafe T26/27 · LALIGA EA SPORTS", "Deportes,Fútbol | 2026 | TP").unwrap();
        assert!(score(&clasico, &none) >= FEATURE_MIN_SCORE);
        assert!(score(&madrid, &none) >= FEATURE_MIN_SCORE);
        assert!(score(&small, &none) < FEATURE_MIN_SCORE);
        let sevillista = Follows { teams: vec!["sevilla".into()], ..Default::default() };
        assert!(score(&small, &sevillista) >= FEATURE_MIN_SCORE, "su equipo, aunque sea pequeño");
        assert!(sevillista.follows(&small));
        assert_eq!(participant("Barça").key, "barcelona");
        assert_eq!(participant("At. Madrid").key, "atletico");
    }

    #[test]
    fn one_event_per_match_with_all_its_channels() {
        let conn = crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        let start = NOW + 3600;
        conn.execute_batch(&format!(
            "INSERT INTO providers (id, name, kind, base_url, created_at) VALUES (1, 'P', 'xtream', 'http://x', 0);
             INSERT INTO categories (id, provider_id, remote_id, kind, name) VALUES (1, 1, '1', 'live', 'DEPORTES'), (2, 1, '2', 'live', 'CINE');
             INSERT INTO channels (id, provider_id, remote_id, category_id, raw_name, display_name, epg_channel_id, quality) VALUES
               (1, 1, 'a', 1, 'DAZN LaLiga', 'DAZN LaLiga', 'dazn', 'HD'), (2, 1, 'b', 1, 'DAZN LaLiga FHD', 'DAZN LaLiga', 'dazn', 'FHD'),
               (3, 1, 'c', 1, 'M+ LaLiga', 'M+ LaLiga', 'mlaliga', 'HD'), (4, 1, 'd', 2, 'Cine', 'Cine', 'cine', NULL);
             INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title, description) VALUES
               ('dazn', {start}, {stop}, 'DIRECTO Jornada 12: Real Madrid - Barcelona T26/27 · LALIGA EA SPORTS', 'Deportes,Fútbol | 2026 | TP'),
               ('mlaliga', {start}, {stop}, 'DIRECTO LALIGA EA SPORTS T26/27 · Jornada 12: R. Madrid - Barça', 'Deportes,Fútbol | 2026 | TP'),
               ('cine', {start}, {stop}, 'Kramer vs. Kramer', 'Cine,Drama | 1979');",
            stop = start + 7200
        ))
        .unwrap();
        let found = events(&conn, NOW, NOW + 86_400, reads::ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].event.name, "Real Madrid - FC Barcelona");
        assert_eq!(found[0].channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["DAZN LaLiga", "M+ LaLiga"]);
        assert_eq!(found[0].channels[0].id, 2, "de DAZN LaLiga, el de FHD");
        assert!(is_big_match(&found[0].event));
        let madrid = EspnTeam { logo: Some("escudo-madrid".into()), ..EspnTeam::named("Real Madrid") };
        let espn = [("soccer/esp.1", madrid), ("soccer/esp.2", EspnTeam::named("Racing Santander"))];
        let offered = suggestions(&found, &espn);
        assert!(offered.iter().any(|f| f.key == "champions"));
        let madrids: Vec<&Followable> = offered.iter().filter(|f| f.kind == "team" && f.label.contains("Madrid") && !f.label.contains("Atl")).collect();
        assert_eq!(madrids.len(), 1, "ni repetido entre los conocidos, ESPN y la guía");
        assert_eq!((madrids[0].key.as_str(), madrids[0].group, madrids[0].logo.as_deref()), ("real-madrid", "LaLiga", Some("escudo-madrid")));
        assert!(offered.iter().any(|f| f.label == "Racing Santander" && f.group == "LaLiga Hypermotion"));
    }

    #[test]
    fn a_team_followed_with_the_espn_name_is_recognised_with_the_guide_name() {
        let follows = Follows { teams: vec!["celta-vigo".into()], team_labels: vec!["Celta Vigo".into()], competitions: vec![] };
        let event = parse_programme("DIRECTO Jornada 12: Celta - Rayo Vallecano · LALIGA EA SPORTS", Some("Deportes,Fútbol"), true, NOW).unwrap();
        assert!(follows.follows(&event));
        let other = parse_programme("DIRECTO Jornada 12: Sevilla - Betis · LALIGA EA SPORTS", Some("Deportes,Fútbol"), true, NOW).unwrap();
        assert!(!follows.follows(&other));
    }
}
