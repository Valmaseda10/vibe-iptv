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
    pub channels: Vec<SportChannel>,
    /// De un equipo o una competición que sigue el perfil.
    pub followed: bool,
    /// Lo grande que es para este perfil (la portada pide 70 o más).
    #[ts(type = "number")]
    pub score: i64,
}

fn to_response(event: SportEvent, follows: &Follows) -> SportEventResponse {
    let followed = follows.follows(&event.event);
    let score = sports::score(&event.event, follows);
    SportEventResponse {
        key: event.key,
        name: event.event.name,
        participants: event.event.participants.into_iter().map(|p| SportParticipant { key: p.key, name: p.name }).collect(),
        competition: event.event.competition,
        competition_key: event.event.competition_key,
        sport: event.event.sport,
        start_ts: event.start_ts,
        stop_ts: event.stop_ts,
        live: event.event.live,
        channels: event.channels.into_iter().map(|c| SportChannel { id: c.id, name: c.name, logo_url: c.logo_url }).collect(),
        followed,
        score,
    }
}

#[derive(Deserialize)]
struct SportsQuery {
    profile_id: i64,
    days: Option<i64>,
}

fn upcoming(conn: &rusqlite::Connection, profile_id: i64, days: i64) -> crate::db::Result<Vec<SportEventResponse>> {
    let now = now_epoch();
    let access = reads::profile_access(conn, profile_id)?;
    let follows = sports::follows(conn, profile_id)?;
    let events = sports::events(conn, now - LIVE_LOOKBACK_S, now + days.clamp(1, MAX_DAYS) * 86_400, access)?;
    Ok(events.into_iter().filter(|e| e.stop_ts > now).map(|e| to_response(e, &follows)).collect())
}

async fn get_sports(State(state): State<SportsState>, Query(query): Query<SportsQuery>) -> Result<Json<Vec<SportEventResponse>>, SportsError> {
    let days = query.days.unwrap_or(MAX_DAYS);
    Ok(Json(with_db(state.db_path, move |conn| upcoming(conn, query.profile_id, days)).await?))
}

#[derive(Deserialize)]
struct ProfileQuery {
    profile_id: i64,
}

/// El partido de la portada: el más grande para el perfil que esté en juego o empiece en
/// las próximas horas (`sports::FEATURE_BEFORE_S`); antes, el que ya ha empezado.
async fn get_featured(State(state): State<SportsState>, Query(query): Query<ProfileQuery>) -> Result<Json<Option<SportEventResponse>>, SportsError> {
    let featured = with_db(state.db_path, move |conn| {
        let now = now_epoch();
        let mut candidates: Vec<SportEventResponse> = upcoming(conn, query.profile_id, 1)?
            .into_iter()
            .filter(|e| e.score >= sports::FEATURE_MIN_SCORE && e.start_ts - sports::FEATURE_BEFORE_S <= now && now < e.stop_ts)
            .collect();
        candidates.sort_by_key(|e| (e.start_ts > now, -e.score, e.start_ts));
        Ok(candidates.into_iter().next())
    })
    .await?;
    Ok(Json(featured))
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
