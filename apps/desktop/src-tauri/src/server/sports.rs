//! Deportes (F17, parte 3): `GET /api/sports` (los eventos de la guía de los próximos días),
//! `GET /api/sports/featured` (el partido de la portada de Inicio) y lo que sigue cada
//! perfil (`/api/sports/follows`, `/api/sports/followables`). Todo sale de la guía ya
//! guardada (`catalog::sports`): nada se pide al proveedor.

use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::collections::HashSet;

use crate::catalog::live_sports::{self, Boards, EspnTeam, Verdict};
use crate::catalog::sports::{self, Follows, SportEvent};
use crate::db::{now_epoch, reads, schema, DbError};
use crate::sync::{engine, outbox};

/// Lo más lejos que se mira la guía.
const MAX_DAYS: i64 = 7;
/// Un evento empezado hace más de esto ya no sale (la guía a veces no tiene el final).
const LIVE_LOOKBACK_S: i64 = 4 * 3600;

#[derive(Clone)]
pub struct SportsState {
    pub db_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum SportsError {
    #[error("petición no válida: {0}")]
    BadRequest(String),
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for SportsError {
    fn into_response(self) -> Response {
        let status = match self {
            SportsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SportsError::Db(_) | SportsError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub fn sports_router(state: SportsState) -> Router {
    Router::new()
        .route("/api/sports", get(get_sports))
        .route("/api/sports/featured", get(get_featured))
        .route("/api/sports/follows", get(get_follows).post(post_follow).delete(delete_follow))
        .route("/api/sports/followables", get(get_followables))
        .route("/api/sports/home-preference", get(get_home_preference).post(post_home_preference))
        .with_state(state)
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, SportsError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| SportsError::Internal)?
    .map_err(SportsError::Db)
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportParticipant {
    pub key: String,
    pub name: String,
    /// Escudo de verdad (ESPN), si se conoce.
    pub logo: Option<String>,
    /// Color del equipo («#a50044»), para el fondo de la portada.
    pub color: Option<String>,
    pub score: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportChannel {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub logo_url: Option<String>,
}

/// Un partido o una carrera, con todos los canales que lo emiten.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportEventResponse {
    /// Estable para el mismo evento (avisos, recordatorios).
    pub key: String,
    pub name: String,
    pub participants: Vec<SportParticipant>,
    pub competition: Option<String>,
    pub competition_key: Option<String>,
    /// `futbol`, `baloncesto`, `tenis`, `motor`...
    pub sport: Option<String>,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
    /// La guía lo da como directo (no una repetición).
    pub live: bool,
    /// Lo que se sabe de verdad: `live` (en juego), `scheduled`, `finished`, `replay`
    /// (diferido: ese partido no se juega hoy), `on_air` (se emite y no se sabe si es en
    /// directo) o `upcoming` (aún no empieza y no se sabe más).
    pub status: String,
    /// «67'», «Descanso», «Final».
    pub clock: Option<String>,
    pub league_logo: Option<String>,
    pub channels: Vec<SportChannel>,
    /// De un equipo o una competición que sigue el perfil.
    pub followed: bool,
    /// Lo grande que es para este perfil (la portada pide 70 o más).
    #[ts(type = "number")]
    pub score: i64,
}

fn hex_color(team: &EspnTeam) -> Option<String> {
    // Un blanco (Madrid) no sirve de fondo: entonces el alternativo.
    let pick = match team.color.as_deref() {
        Some(c) if !c.eq_ignore_ascii_case("ffffff") && !c.is_empty() => Some(c),
        _ => team.alternate_color.as_deref().or(team.color.as_deref()),
    };
    pick.filter(|c| c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit())).map(|c| format!("#{c}"))
}

fn to_response(event: SportEvent, follows: &Follows, favorites: &HashSet<i64>, boards: &Boards, now: i64) -> SportEventResponse {
    let followed = follows.follows(&event.event);
    let score = sports::score(&event.event, follows);
    let verdict = live_sports::verify(&event, boards);
    let on_now = event.start_ts <= now && now < event.stop_ts;
    let status = match &verdict {
        Verdict::Live(_) => "live",
        Verdict::Scheduled(_) => "scheduled",
        Verdict::Finished(_) => "finished",
        Verdict::Replay => "replay",
        // Sin ESPN manda la guía: «DIRECTO» y en emisión es directo.
        Verdict::Unknown if on_now && event.event.live => "live",
        Verdict::Unknown if on_now => "on_air",
        Verdict::Unknown => "upcoming",
    };
    let espn = match &verdict {
        Verdict::Live(e) | Verdict::Scheduled(e) | Verdict::Finished(e) => Some(e),
        _ => None,
    };
    let league = event.event.competition_key.as_deref().and_then(live_sports::espn_league);
    let participants = event
        .event
        .participants
        .into_iter()
        .map(|p| {
            let team = boards.team(&p.name, league, espn);
            SportParticipant {
                logo: team.and_then(|t| t.logo.clone()),
                color: team.and_then(hex_color),
                score: team.and_then(|t| t.score.clone()).filter(|_| matches!(status, "live" | "finished")),
                key: p.key,
                name: p.name,
            }
        })
        .collect();
    // El mejor canal primero: uno de tus favoritos y, entre ellos, el de mejor calidad.
    let mut channels = event.channels;
    channels.sort_by_key(|c| (!favorites.contains(&c.id), -sports::quality_rank(c.quality.as_deref())));
    SportEventResponse {
        key: event.key,
        name: event.event.name,
        participants,
        competition: event.event.competition,
        competition_key: event.event.competition_key,
        sport: event.event.sport,
        start_ts: event.start_ts,
        stop_ts: event.stop_ts,
        live: event.event.live,
        status: status.to_string(),
        clock: espn.and_then(|e| e.detail.clone()).filter(|_| matches!(status, "live" | "finished")),
        league_logo: espn.and_then(|e| e.league_logo.clone()),
        channels: channels.into_iter().map(|c| SportChannel { id: c.id, name: c.name, logo_url: c.logo_url }).collect(),
        followed,
        score,
    }
}

#[derive(Deserialize)]
struct SportsQuery {
    profile_id: i64,
    days: Option<i64>,
}

/// Lo de la guía del perfil, y lo que hace falta para enseñarlo.
struct Gathered {
    events: Vec<SportEvent>,
    follows: Follows,
    favorites: HashSet<i64>,
    sports_home: String,
}

/// Los eventos de los próximos días, contrastados con ESPN (`catalog::live_sports`).
async fn upcoming(db_path: PathBuf, profile_id: i64, days: i64) -> Result<(Vec<SportEventResponse>, String), SportsError> {
    let now = now_epoch();
    let gathered = with_db(db_path, move |conn| {
        let access = reads::profile_access(conn, profile_id)?;
        let follows = sports::follows(conn, profile_id)?;
        let events = sports::events(conn, now - LIVE_LOOKBACK_S, now + days.clamp(1, MAX_DAYS) * 86_400, access)?;
        let mut stmt = conn.prepare("SELECT item_id FROM favorites WHERE profile_id = ?1 AND item_kind = 'channel'")?;
        let favorites = stmt.query_map(rusqlite::params![profile_id], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<HashSet<_>>>()?;
        let sports_home: String = conn
            .query_row("SELECT sports_home FROM profiles WHERE id = ?1", rusqlite::params![profile_id], |r| r.get(0))
            .unwrap_or_else(|_| "mine_and_big".to_string());
        Ok(Gathered { events: events.into_iter().filter(|e| e.stop_ts > now).collect(), follows, favorites, sports_home })
    })
    .await?;
    let boards = live_sports::boards_for(&gathered.events, now).await;
    let responses = gathered.events.into_iter().map(|e| to_response(e, &gathered.follows, &gathered.favorites, &boards, now)).collect();
    Ok((responses, gathered.sports_home))
}

async fn get_sports(State(state): State<SportsState>, Query(query): Query<SportsQuery>) -> Result<Json<Vec<SportEventResponse>>, SportsError> {
    let days = query.days.unwrap_or(MAX_DAYS);
    Ok(Json(upcoming(state.db_path, query.profile_id, days).await?.0))
}

#[derive(Deserialize)]
struct ProfileQuery {
    profile_id: i64,
}

/// El partido de la portada (pedido por el usuario el 2026-11-02): solo uno que se esté
/// jugando de verdad ahora (nunca una repetición) y de los que quiere el perfil en
/// «Partidos en Inicio»: sus equipos, o sus equipos y los grandes, o ninguno. Antes, los
/// suyos; luego, el más grande.
async fn get_featured(State(state): State<SportsState>, Query(query): Query<ProfileQuery>) -> Result<Json<Option<SportEventResponse>>, SportsError> {
    let (events, preference) = upcoming(state.db_path, query.profile_id, 1).await?;
    let mut candidates: Vec<SportEventResponse> = events
        .into_iter()
        .filter(|e| e.status == "live")
        .filter(|e| match preference.as_str() {
            "never" => false,
            "mine" => e.followed,
            _ => e.followed || is_big(e),
        })
        .collect();
    candidates.sort_by_key(|e| (!e.followed, -e.score, e.start_ts));
    Ok(Json(candidates.into_iter().next()))
}

fn is_big(event: &SportEventResponse) -> bool {
    let parsed = sports::ParsedEvent {
        name: event.name.clone(),
        participants: event.participants.iter().map(|p| sports::Participant { key: p.key.clone(), name: p.name.clone() }).collect(),
        competition: event.competition.clone(),
        competition_key: event.competition_key.clone(),
        sport: event.sport.clone(),
        live: event.live,
    };
    sports::is_big_match(&parsed)
}

/// «Partidos en Inicio» del perfil: `mine`, `mine_and_big` o `never`.
#[derive(Serialize, Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportsHomePreference {
    #[ts(type = "number")]
    pub profile_id: i64,
    pub value: String,
}

async fn get_home_preference(State(state): State<SportsState>, Query(query): Query<ProfileQuery>) -> Result<Json<SportsHomePreference>, SportsError> {
    let value = with_db(state.db_path, move |conn| {
        Ok(conn.query_row("SELECT sports_home FROM profiles WHERE id = ?1", rusqlite::params![query.profile_id], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "mine_and_big".into()))
    })
    .await?;
    Ok(Json(SportsHomePreference { profile_id: query.profile_id, value }))
}

async fn post_home_preference(State(state): State<SportsState>, Json(input): Json<SportsHomePreference>) -> Result<StatusCode, SportsError> {
    if !crate::db::cloud::SPORTS_HOME_VALUES.contains(&input.value.as_str()) {
        return Err(SportsError::BadRequest("value debe ser mine, mine_and_big o never".into()));
    }
    with_db(state.db_path, move |conn| {
        conn.execute("UPDATE profiles SET sports_home = ?1, updated_at = ?2 WHERE id = ?3", rusqlite::params![input.value, now_epoch(), input.profile_id])?;
        // Viaja con el perfil, a sus otros aparatos.
        outbox::enqueue_profile(conn, input.profile_id)
    })
    .await?;
    engine::wake();
    Ok(StatusCode::NO_CONTENT)
}

/// Algo que sigue (o puede seguir) un perfil.
#[derive(Serialize, Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportFollow {
    /// `team` o `competition`.
    pub kind: String,
    pub key: String,
    pub label: String,
    pub sport: Option<String>,
    pub followed: bool,
}

async fn get_follows(State(state): State<SportsState>, Query(query): Query<ProfileQuery>) -> Result<Json<Vec<SportFollow>>, SportsError> {
    let rows = with_db(state.db_path, move |conn| {
        let mut stmt = conn.prepare("SELECT kind, key, label FROM sport_follows WHERE profile_id = ?1 ORDER BY created_at")?;
        let rows = stmt
            .query_map(rusqlite::params![query.profile_id], |r| Ok(SportFollow { kind: r.get(0)?, key: r.get(1)?, label: r.get(2)?, sport: None, followed: true }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await?;
    Ok(Json(rows))
}

/// Lo que se ofrece en «Mis equipos»: los conocidos y quien sale en la guía, con lo que ya
/// se sigue marcado (y lo seguido que ya no sale en la guía, también).
async fn get_followables(State(state): State<SportsState>, Query(query): Query<ProfileQuery>) -> Result<Json<Vec<SportFollow>>, SportsError> {
    let list = with_db(state.db_path, move |conn| {
        let now = now_epoch();
        let access = reads::profile_access(conn, query.profile_id)?;
        let events = sports::events(conn, now - LIVE_LOOKBACK_S, now + MAX_DAYS * 86_400, access)?;
        let follows = sports::follows(conn, query.profile_id)?;
        let mut out: Vec<SportFollow> = sports::suggestions(&events)
            .into_iter()
            .map(|f| {
                let followed = if f.kind == "team" { follows.teams.contains(&f.key) } else { follows.competitions.contains(&f.key) };
                SportFollow { kind: f.kind.to_string(), key: f.key, label: f.label, sport: f.sport, followed }
            })
            .collect();
        let mut stmt = conn.prepare("SELECT kind, key, label FROM sport_follows WHERE profile_id = ?1")?;
        let saved = stmt.query_map(rusqlite::params![query.profile_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in saved {
            let (kind, key, label) = row?;
            if !out.iter().any(|f| f.kind == kind && f.key == key) {
                out.insert(0, SportFollow { kind, key, label, sport: None, followed: true });
            }
        }
        Ok(out)
    })
    .await?;
    Ok(Json(list))
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SportFollowInput {
    #[ts(type = "number")]
    pub profile_id: i64,
    pub kind: String,
    pub key: String,
    pub label: String,
}

async fn post_follow(State(state): State<SportsState>, Json(input): Json<SportFollowInput>) -> Result<StatusCode, SportsError> {
    if !matches!(input.kind.as_str(), "team" | "competition") || input.key.trim().is_empty() {
        return Err(SportsError::BadRequest("kind debe ser team o competition, con key".into()));
    }
    with_db(state.db_path, move |conn| {
        conn.execute(
            "INSERT INTO sport_follows (profile_id, kind, key, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_id, kind, key) DO UPDATE SET label = excluded.label",
            rusqlite::params![input.profile_id, input.kind, input.key, input.label, now_epoch()],
        )?;
        outbox::enqueue_sport_follow(conn, input.profile_id, &input.kind, &input.key, &input.label, false)
    })
    .await?;
    engine::wake();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct FollowKey {
    profile_id: i64,
    kind: String,
    key: String,
}

async fn delete_follow(State(state): State<SportsState>, Query(key): Query<FollowKey>) -> Result<StatusCode, SportsError> {
    with_db(state.db_path, move |conn| {
        let label: Option<String> = conn
            .query_row("SELECT label FROM sport_follows WHERE profile_id = ?1 AND kind = ?2 AND key = ?3", rusqlite::params![key.profile_id, key.kind, key.key], |r| r.get(0))
            .ok();
        conn.execute("DELETE FROM sport_follows WHERE profile_id = ?1 AND kind = ?2 AND key = ?3", rusqlite::params![key.profile_id, key.kind, key.key])?;
        outbox::enqueue_sport_follow(conn, key.profile_id, &key.kind, &key.key, label.as_deref().unwrap_or(""), true)
    })
    .await?;
    engine::wake();
    Ok(StatusCode::NO_CONTENT)
}
