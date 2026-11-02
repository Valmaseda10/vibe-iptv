//! Datos reales de los partidos (F17, ampliación del 2026-11-02, pedida por el usuario):
//! escudos, colores, marcador, minuto y, sobre todo, si un partido de la guía se está
//! jugando de verdad o es una repetición (Real Madrid TV echa partidos de hace semanas como
//! si fueran nuevos).
//!
//! Sale de los marcadores públicos de ESPN (`site.api.espn.com`, sin clave). No es un
//! servicio oficial: si algún día cambia o no responde, todo sigue como antes (escudos
//! dibujados y la marca «DIRECTO» de la guía). Solo se piden las ligas y los días que tienen
//! partidos en la guía, un día por petición (ESPN no admite rangos); hoy se refresca cada
//! dos minutos (marcador) y el resto cada seis horas.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::sports::{normalize, participant, SportEvent};

const BASE: &str = "https://site.api.espn.com/apis/site/v2/sports";
const TIMEOUT: Duration = Duration::from_secs(6);
/// Hoy y ayer cambian (marcador, minuto); el resto apenas.
const LIVE_TTL: Duration = Duration::from_secs(120);
const LATER_TTL: Duration = Duration::from_secs(6 * 3600);
/// Los equipos de una liga (escudos y colores) apenas cambian.
const TEAMS_TTL: Duration = Duration::from_secs(24 * 3600);
/// Tras un fallo, no se vuelve a pedir lo mismo enseguida.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(5 * 60);
/// El mismo partido en la guía y en ESPN: como mucho esto de diferencia al empezar (la
/// guía suele empezar antes, con la previa).
const SAME_MATCH_WINDOW_S: i64 = 3 * 3600;

/// Liga de ESPN de cada competición que reconoce `catalog::sports`.
pub fn espn_league(competition_key: &str) -> Option<&'static str> {
    Some(match competition_key {
        "laliga" => "soccer/esp.1",
        "laliga2" => "soccer/esp.2",
        "copa-del-rey" => "soccer/esp.copa_del_rey",
        "champions" => "soccer/uefa.champions",
        "europa-league" => "soccer/uefa.europa",
        "premier" => "soccer/eng.1",
        "serie-a" => "soccer/ita.1",
        "bundesliga" => "soccer/ger.1",
        "ligue-1" => "soccer/fra.1",
        "mundial" => "soccer/fifa.world",
        "eurocopa" => "soccer/uefa.euro",
        "nba" => "basketball/nba",
        "nfl" => "football/nfl",
        "mlb" => "baseball/mlb",
        _ => return None,
    })
}

/// Un equipo según ESPN.
#[derive(Debug, Clone, PartialEq)]
pub struct EspnTeam {
    pub name: String,
    pub short_name: Option<String>,
    pub abbreviation: Option<String>,
    pub logo: Option<String>,
    /// Hexadecimal sin «#».
    pub color: Option<String>,
    pub alternate_color: Option<String>,
    pub score: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EspnState {
    Scheduled,
    Live,
    Finished,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EspnEvent {
    pub league: &'static str,
    pub league_name: Option<String>,
    pub league_logo: Option<String>,
    pub start_ts: i64,
    pub state: EspnState,
    /// «67'», «Descanso», «Final».
    pub detail: Option<String>,
    pub home: EspnTeam,
    pub away: EspnTeam,
}

// ─────────────── Respuesta de ESPN ───────────────

#[derive(Deserialize)]
struct Board {
    #[serde(default)]
    leagues: Vec<LeagueWire>,
    #[serde(default)]
    events: Vec<EventWire>,
}

#[derive(Deserialize)]
struct LeagueWire {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    logos: Vec<LogoWire>,
}

#[derive(Deserialize)]
struct LogoWire {
    href: String,
}

#[derive(Deserialize)]
struct TeamsListWire {
    #[serde(default)]
    sports: Vec<TeamsSportWire>,
}

#[derive(Deserialize)]
struct TeamsSportWire {
    #[serde(default)]
    leagues: Vec<TeamsLeagueWire>,
}

#[derive(Deserialize)]
struct TeamsLeagueWire {
    #[serde(default)]
    teams: Vec<TeamsEntryWire>,
}

#[derive(Deserialize)]
struct TeamsEntryWire {
    team: ListedTeamWire,
}

#[derive(Deserialize)]
struct ListedTeamWire {
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "shortDisplayName")]
    short_display_name: Option<String>,
    #[serde(default)]
    abbreviation: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default, rename = "alternateColor")]
    alternate_color: Option<String>,
    #[serde(default)]
    logos: Vec<LogoWire>,
}

fn parse_teams(json: &str) -> Option<Vec<EspnTeam>> {
    let list: TeamsListWire = serde_json::from_str(json).ok()?;
    let teams = list.sports.into_iter().flat_map(|s| s.leagues).flat_map(|l| l.teams);
    Some(
        teams
            .map(|t| EspnTeam {
                name: t.team.display_name,
                short_name: t.team.short_display_name,
                abbreviation: t.team.abbreviation,
                logo: t.team.logos.into_iter().next().map(|l| l.href),
                color: t.team.color,
                alternate_color: t.team.alternate_color,
                score: None,
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct EventWire {
    date: String,
    status: StatusWire,
    #[serde(default)]
    competitions: Vec<CompetitionWire>,
}

#[derive(Deserialize)]
struct StatusWire {
    #[serde(rename = "type")]
    kind: StatusTypeWire,
    #[serde(default, rename = "displayClock")]
    display_clock: Option<String>,
}

#[derive(Deserialize)]
struct StatusTypeWire {
    state: String,
    #[serde(default, rename = "shortDetail")]
    short_detail: Option<String>,
}

#[derive(Deserialize)]
struct CompetitionWire {
    #[serde(default)]
    competitors: Vec<CompetitorWire>,
}

#[derive(Deserialize)]
struct CompetitorWire {
    #[serde(default, rename = "homeAway")]
    home_away: String,
    #[serde(default)]
    score: Option<String>,
    team: TeamWire,
}

#[derive(Deserialize)]
struct TeamWire {
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "shortDisplayName")]
    short_display_name: Option<String>,
    #[serde(default)]
    abbreviation: Option<String>,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default, rename = "alternateColor")]
    alternate_color: Option<String>,
}

/// «2026-11-02T20:00Z» → epoch.
fn parse_espn_date(date: &str) -> Option<i64> {
    let date = date.trim_end_matches('Z');
    let (day, time) = date.split_once('T')?;
    let mut d = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, dd) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi) = (t.next()??, t.next().flatten().unwrap_or(0));
    let days = crate::util::civil_time::days_from_civil(i32::try_from(y).ok()?, u32::try_from(m).ok()?, u32::try_from(dd).ok()?)?;
    Some(days * 86_400 + h * 3600 + mi * 60)
}

/// «Halftime» → «Descanso», «FT» → «Final»; el minuto («67'») tal cual.
fn spanish_detail(state: EspnState, short_detail: Option<&str>, clock: Option<&str>) -> Option<String> {
    let detail = short_detail.unwrap_or("").trim();
    match state {
        EspnState::Finished => Some("Final".into()),
        EspnState::Scheduled => None,
        EspnState::Live => {
            let lower = detail.to_lowercase();
            if lower.contains("half") && !lower.contains("1st") && !lower.contains("2nd") {
                Some("Descanso".into())
            } else if detail.ends_with('\'') || detail.contains('\'') {
                Some(detail.to_string())
            } else {
                clock.filter(|c| !c.is_empty() && *c != "0:00").map(str::to_string).or_else(|| Some("En juego".into()))
            }
        }
    }
}

fn parse_board(league: &'static str, json: &str) -> Option<Vec<EspnEvent>> {
    let board: Board = serde_json::from_str(json).ok()?;
    let league_name = board.leagues.first().and_then(|l| l.name.clone());
    let league_logo = board.leagues.first().and_then(|l| l.logos.first()).map(|l| l.href.clone());
    let mut out = Vec::new();
    for event in board.events {
        let Some(start_ts) = parse_espn_date(&event.date) else { continue };
        let state = match event.status.kind.state.as_str() {
            "in" => EspnState::Live,
            "post" => EspnState::Finished,
            _ => EspnState::Scheduled,
        };
        let Some(competition) = event.competitions.into_iter().next() else { continue };
        let mut home = None;
        let mut away = None;
        for c in competition.competitors {
            let team = EspnTeam {
                name: c.team.display_name,
                short_name: c.team.short_display_name,
                abbreviation: c.team.abbreviation,
                logo: c.team.logo,
                color: c.team.color,
                alternate_color: c.team.alternate_color,
                score: c.score,
            };
            if c.home_away == "away" {
                away = Some(team);
            } else {
                home = Some(team);
            }
        }
        let (Some(home), Some(away)) = (home, away) else { continue };
        out.push(EspnEvent {
            league,
            league_name: league_name.clone(),
            league_logo: league_logo.clone(),
            start_ts,
            state,
            detail: spanish_detail(state, event.status.kind.short_detail.as_deref(), event.status.display_clock.as_deref()),
            home,
            away,
        });
    }
    Some(out)
}

// ─────────────── Descarga, con caché ───────────────

#[derive(Default)]
struct Cache {
    boards: HashMap<(&'static str, String), (Instant, Vec<EspnEvent>)>,
    failed: HashMap<(&'static str, String), Instant>,
    teams: HashMap<&'static str, (Instant, Vec<EspnTeam>)>,
    teams_failed: HashMap<&'static str, Instant>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));

fn yyyymmdd(day: i64) -> String {
    let (y, m, d) = super::watch_time::civil_from_days(day);
    format!("{y:04}{m:02}{d:02}")
}

async fn fetch_board(client: &reqwest::Client, league: &'static str, day: &str) -> Option<Vec<EspnEvent>> {
    let url = format!("{BASE}/{league}/scoreboard?dates={day}");
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    parse_board(league, &text)
}

async fn fetch_teams(client: &reqwest::Client, league: &'static str) -> Option<Vec<EspnTeam>> {
    let response = client.get(format!("{BASE}/{league}/teams")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_teams(&response.text().await.ok()?)
}

/// Lo que dice ESPN de cada día y liga pedidos: `Some` si se sabe (aunque no haya
/// partidos), `None` si no se pudo saber.
#[derive(Default)]
pub struct Boards {
    pub events: Vec<EspnEvent>,
    /// Ligas y días con respuesta.
    pub known: HashSet<(&'static str, i64)>,
    /// Los equipos de cada liga: escudos de todos (también de los que no juegan hoy) y
    /// para saber si ESPN conoce a un equipo.
    pub teams: Vec<(&'static str, EspnTeam)>,
}

impl Boards {
    /// Escudo y colores de un rival de la guía: del partido de ESPN si lo hay, si no de
    /// la lista de su liga, y si no de cualquier liga.
    pub fn team<'a>(&'a self, name: &str, league: Option<&'static str>, event: Option<&'a EspnEvent>) -> Option<&'a EspnTeam> {
        if let Some(found) = event.and_then(|e| [&e.home, &e.away].into_iter().find(|t| same_team(name, t))) {
            return Some(found);
        }
        let in_league = league.and_then(|l| self.teams.iter().find(|(tl, t)| *tl == l && same_team(name, t)));
        in_league.or_else(|| self.teams.iter().find(|(_, t)| same_team(name, t))).map(|(_, t)| t)
    }
}

/// Los marcadores de las ligas y días de estos eventos de la guía (el día de cada uno y el
/// anterior: ESPN cuenta los días en horario de EE. UU.).
pub async fn boards_for(events: &[SportEvent], now: i64) -> Boards {
    let today = now.div_euclid(86_400);
    let mut wanted: HashSet<(&'static str, i64)> = HashSet::new();
    for event in events {
        let Some(league) = event.event.competition_key.as_deref().and_then(espn_league) else { continue };
        let day = event.start_ts.div_euclid(86_400);
        wanted.insert((league, day));
        wanted.insert((league, day - 1));
    }
    let mut out = Boards::default();
    let mut to_fetch = Vec::new();
    let leagues: HashSet<&'static str> = wanted.iter().map(|(league, _)| *league).collect();
    let mut teams_to_fetch = Vec::new();
    if let Ok(cache) = CACHE.lock() {
        for league in &leagues {
            match cache.teams.get(league) {
                Some((at, teams)) if at.elapsed() < TEAMS_TTL => out.teams.extend(teams.iter().map(|t| (*league, t.clone()))),
                Some((_, teams)) if cache.teams_failed.get(league).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) => {
                    out.teams.extend(teams.iter().map(|t| (*league, t.clone())));
                }
                None if cache.teams_failed.get(league).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) => {}
                _ => teams_to_fetch.push(*league),
            }
        }
        for (league, day) in &wanted {
            let key = (*league, yyyymmdd(*day));
            let ttl = if (*day - today).abs() <= 1 { LIVE_TTL } else { LATER_TTL };
            match cache.boards.get(&key) {
                Some((at, events)) if at.elapsed() < ttl => {
                    out.events.extend(events.iter().cloned());
                    out.known.insert((*league, *day));
                }
                _ if cache.failed.get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) => {
                    // Falló hace poco: lo viejo que hubiera, si hay.
                    if let Some((_, events)) = cache.boards.get(&key) {
                        out.events.extend(events.iter().cloned());
                        out.known.insert((*league, *day));
                    }
                }
                _ => to_fetch.push((*league, *day)),
            }
        }
    }
    if to_fetch.is_empty() && teams_to_fetch.is_empty() {
        return out;
    }
    let Ok(client) = reqwest::Client::builder().timeout(TIMEOUT).user_agent("Mozilla/5.0 VIBE-IPTV").build() else { return out };
    let fetched_teams = futures_util::future::join_all(teams_to_fetch.iter().map(|league| {
        let client = &client;
        async move { (*league, fetch_teams(client, league).await) }
    }))
    .await;
    if let Ok(mut cache) = CACHE.lock() {
        for (league, result) in fetched_teams {
            match result {
                Some(teams) => {
                    out.teams.extend(teams.iter().map(|t| (league, t.clone())));
                    cache.teams_failed.remove(league);
                    cache.teams.insert(league, (Instant::now(), teams));
                }
                None => {
                    cache.teams_failed.insert(league, Instant::now());
                    if let Some((_, teams)) = cache.teams.get(league) {
                        out.teams.extend(teams.iter().map(|t| (league, t.clone())));
                    }
                }
            }
        }
    }
    let fetched = futures_util::future::join_all(to_fetch.iter().map(|(league, day)| {
        let client = &client;
        async move { (*league, *day, fetch_board(client, league, &yyyymmdd(*day)).await) }
    }))
    .await;
    if let Ok(mut cache) = CACHE.lock() {
        for (league, day, result) in fetched {
            let key = (league, yyyymmdd(day));
            match result {
                Some(events) => {
                    out.events.extend(events.iter().cloned());
                    out.known.insert((league, day));
                    cache.failed.remove(&key);
                    cache.boards.insert(key, (Instant::now(), events));
                }
                None => {
                    cache.failed.insert(key.clone(), Instant::now());
                    if let Some((_, events)) = cache.boards.get(&key) {
                        out.events.extend(events.iter().cloned());
                        out.known.insert((league, day));
                    }
                }
            }
        }
    }
    let failed = to_fetch.len().saturating_sub(out.known.len().min(to_fetch.len()));
    tracing::debug!(pedidos = to_fetch.len(), failed, "marcadores de ESPN");
    out
}

// ─────────────── Guía contra ESPN ───────────────

/// Palabras que no distinguen un equipo («FC», «CF», «Club»...).
const CLUB_WORDS: &[&str] = &["fc", "cf", "cd", "ud", "sd", "rc", "rcd", "ca", "ac", "as", "ss", "afc", "sc", "club", "de", "del", "la"];

fn significant(name: &str) -> String {
    normalize(name).split(' ').filter(|w| !CLUB_WORDS.contains(w)).collect::<Vec<_>>().join(" ")
}

/// Si un rival de la guía es este equipo de ESPN.
pub fn same_team(guide_name: &str, team: &EspnTeam) -> bool {
    let guide_key = participant(guide_name).key;
    let names = [Some(team.name.as_str()), team.short_name.as_deref()];
    for name in names.into_iter().flatten() {
        let espn_key = participant(name).key;
        if espn_key == guide_key {
            return true;
        }
        // Dos equipos conocidos distintos («Madrid» y «Atlético Madrid») no se confunden por
        // compartir una palabra.
        if super::sports::is_known_team(&guide_key) && super::sports::is_known_team(&espn_key) {
            continue;
        }
        let (a, b) = (significant(guide_name), significant(name));
        if a.is_empty() || b.is_empty() {
            continue;
        }
        if a == b {
            return true;
        }
        let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
        // «Leipzig» en «RB Leipzig», «Celta» en «Celta Vigo»: por palabras enteras.
        if short.len() >= 4 && format!(" {long} ").contains(&format!(" {short} ")) {
            return true;
        }
    }
    false
}

/// Lo que se sabe de verdad de un partido de la guía.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// ESPN dice que se está jugando ahora.
    Live(EspnEvent),
    /// Aún no ha empezado (según ESPN).
    Scheduled(EspnEvent),
    /// Ya terminó (la guía sigue con el partido, o es la repetición del de hoy).
    Finished(EspnEvent),
    /// ESPN conoce la competición ese día y ese partido no se juega: es una repetición.
    Replay,
    /// No se sabe (competición que ESPN no cubre, o sin respuesta): manda la guía.
    Unknown,
}

pub fn verify(event: &SportEvent, boards: &Boards) -> Verdict {
    let [home, away] = event.event.participants.as_slice() else { return Verdict::Unknown };
    let found = boards.events.iter().find(|e| {
        (e.start_ts - event.start_ts).abs() <= SAME_MATCH_WINDOW_S
            && ((same_team(&home.name, &e.home) && same_team(&away.name, &e.away)) || (same_team(&home.name, &e.away) && same_team(&away.name, &e.home)))
    });
    if let Some(found) = found {
        return match found.state {
            EspnState::Live => Verdict::Live(found.clone()),
            EspnState::Scheduled => Verdict::Scheduled(found.clone()),
            EspnState::Finished => Verdict::Finished(found.clone()),
        };
    }
    let Some(league) = event.event.competition_key.as_deref().and_then(espn_league) else { return Verdict::Unknown };
    let day = event.start_ts.div_euclid(86_400);
    if !boards.known.contains(&(league, day)) {
        return Verdict::Unknown;
    }
    // La competición ese día se sabe y el partido no está. Si ese día no juega nadie o
    // alguno de los dos sí juega (contra otro, u otro día), es una repetición. Si ninguno
    // de los dos sale nunca, ESPN puede no tenerlos (equipos pequeños de Copa): no se sabe.
    let league_events: Vec<&EspnEvent> = boards.events.iter().filter(|e| e.league == league).collect();
    let empty_that_day = !league_events.iter().any(|e| e.start_ts.div_euclid(86_400) == day);
    let known_team = boards.events.iter().any(|e| [&e.home, &e.away].iter().any(|t| same_team(&home.name, t) || same_team(&away.name, t)))
        || boards.teams.iter().any(|(l, t)| *l == league && (same_team(&home.name, t) || same_team(&away.name, t)));
    if empty_that_day || known_team {
        Verdict::Replay
    } else {
        Verdict::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::sports::{EventChannel, ParsedEvent};

    const BOARD: &str = r#"{"leagues":[{"name":"Spanish LALIGA","logos":[{"href":"https://a.espncdn.com/laliga.png"}]}],
      "events":[
        {"date":"2026-11-02T20:00Z","status":{"type":{"state":"in","shortDetail":"67'"},"displayClock":"67'"},
         "competitions":[{"competitors":[
           {"homeAway":"home","score":"2","team":{"displayName":"Real Madrid","shortDisplayName":"Real Madrid","abbreviation":"RMA","logo":"https://a.espncdn.com/86.png","color":"ffffff","alternateColor":"2b3a86"}},
           {"homeAway":"away","score":"1","team":{"displayName":"Barcelona","shortDisplayName":"Barcelona","abbreviation":"BAR","logo":"https://a.espncdn.com/83.png","color":"a50044","alternateColor":"004d98"}}]}]},
        {"date":"2026-11-02T17:30Z","status":{"type":{"state":"post","shortDetail":"FT"}},
         "competitions":[{"competitors":[
           {"homeAway":"home","score":"0","team":{"displayName":"Getafe"}},
           {"homeAway":"away","score":"0","team":{"displayName":"Sevilla"}}]}]}
      ]}"#;

    fn guide(home: &str, away: &str, key: &str, start_ts: i64) -> SportEvent {
        SportEvent {
            key: String::new(),
            event: ParsedEvent {
                name: format!("{home} - {away}"),
                participants: vec![participant(home), participant(away)],
                competition: None,
                competition_key: Some(key.into()),
                sport: Some("futbol".into()),
                live: false,
            },
            start_ts,
            stop_ts: start_ts + 7200,
            channels: vec![EventChannel { id: 1, name: "C".into(), logo_url: None, quality: None }],
        }
    }

    #[test]
    fn parses_the_scoreboard_with_crests_colours_and_minute() {
        let events = parse_board("soccer/esp.1", BOARD).unwrap();
        assert_eq!(events.len(), 2);
        let clasico = &events[0];
        assert_eq!(clasico.state, EspnState::Live);
        assert_eq!(clasico.detail.as_deref(), Some("67'"));
        assert_eq!((clasico.home.score.as_deref(), clasico.away.score.as_deref()), (Some("2"), Some("1")));
        assert_eq!(clasico.away.color.as_deref(), Some("a50044"));
        assert_eq!(clasico.league_logo.as_deref(), Some("https://a.espncdn.com/laliga.png"));
        assert_eq!(events[1].detail.as_deref(), Some("Final"));
        assert_eq!(parse_espn_date("1970-01-02T01:30Z"), Some(86_400 + 5400));
    }

    #[test]
    fn team_names_match_across_guide_and_espn() {
        let team = |name: &str, short: Option<&str>| EspnTeam { name: name.into(), short_name: short.map(Into::into), abbreviation: None, logo: None, color: None, alternate_color: None, score: None };
        assert!(same_team("Leipzig", &team("RB Leipzig", None)));
        assert!(same_team("Barça", &team("Barcelona", None)));
        assert!(same_team("Atlético de Madrid", &team("Atlético Madrid", None)));
        assert!(same_team("Celta", &team("Celta Vigo", None)));
        assert!(same_team("UD Almería", &team("Almería", None)));
        assert!(!same_team("Real Madrid", &team("Real Sociedad", None)), "«Real» no basta");
        assert!(!same_team("Madrid", &team("Atlético Madrid", None)));
    }

    #[test]
    fn live_replay_and_unknown() {
        let board = parse_board("soccer/esp.1", BOARD).unwrap();
        let start = board[0].start_ts;
        let day = start.div_euclid(86_400);
        let boards = Boards { events: board, known: [("soccer/esp.1", day)].into_iter().collect(), teams: Vec::new() };

        // El clásico en la guía empieza media hora antes (previa): es el mismo y está en juego.
        assert!(matches!(verify(&guide("Real Madrid", "Barcelona", "laliga", start - 1800), &boards), Verdict::Live(_)));
        // Orden cambiado también vale.
        assert!(matches!(verify(&guide("Barça", "Real Madrid", "laliga", start), &boards), Verdict::Live(_)));
        // Un Madrid - Leipzig ese día en LaLiga/Champions que ESPN no tiene: repetición.
        assert_eq!(verify(&guide("Real Madrid", "Leipzig", "laliga", start), &boards), Verdict::Replay);
        // La Champions ese día no se ha pedido: no se sabe.
        assert_eq!(verify(&guide("Real Madrid", "Leipzig", "champions", start), &boards), Verdict::Unknown);
        // Dos equipos que ESPN no conoce, un día con partidos de esa liga: no se sabe.
        assert_eq!(verify(&guide("Ourense", "Zamora", "laliga", start), &boards), Verdict::Unknown);
        // Con la lista de equipos de la liga, un partido de dos equipos suyos que no se
        // juega ese día es una repetición aunque no jueguen esos días.
        let teams = parse_teams(r#"{"sports":[{"leagues":[{"teams":[{"team":{"displayName":"Rayo Vallecano","logos":[{"href":"https://a.espncdn.com/101.png"}]}},{"team":{"displayName":"Celta Vigo"}}]}]}]}"#).unwrap();
        let with_teams = Boards { events: boards.events.clone(), known: boards.known.clone(), teams: teams.into_iter().map(|t| ("soccer/esp.1", t)).collect() };
        assert_eq!(verify(&guide("Rayo", "Celta", "laliga", start), &with_teams), Verdict::Replay);
        assert_eq!(with_teams.team("Rayo", Some("soccer/esp.1"), None).and_then(|t| t.logo.as_deref()), Some("https://a.espncdn.com/101.png"));

        // El Getafe - Sevilla ya acabó.
        assert!(matches!(verify(&guide("Getafe", "Sevilla", "laliga", start - 9000), &boards), Verdict::Finished(_)));
    }
}
