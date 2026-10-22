//! Funciones de tele del directo (F13): canal por número, canales recientes, búsqueda en
//! la guía y recordatorios de programa. Todo local: nada de esto toca al proveedor.

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::api::ChannelSummary;
use crate::db::{now_epoch, reads, schema, DbError};

#[derive(Clone)]
pub struct TvState {
    pub db_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum TvError {
    #[error("no encontrado")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for TvError {
    fn into_response(self) -> Response {
        let status = match self {
            TvError::NotFound => StatusCode::NOT_FOUND,
            TvError::BadRequest(_) => StatusCode::BAD_REQUEST,
            TvError::Db(_) | TvError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, TvError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| TvError::Internal)?
    .map_err(TvError::Db)
}

pub fn tv_router(state: TvState) -> Router {
    Router::new()
        .route("/api/live/channels/by-number/:number", get(get_channel_by_number))
        .route("/api/live/recent", get(get_recent_channels))
        .route("/api/epg/search", get(get_epg_search))
        .route("/api/reminders", get(get_reminders).post(post_reminder).delete(delete_reminder))
        .with_state(state)
}

#[derive(Deserialize)]
struct ProfileQuery {
    profile_id: i64,
}

/// Teclear un número en el directo (F13): el canal con ese número del proveedor. 404 si
/// no hay ninguno (o es de adultos y el perfil no puede verlo).
#[derive(Deserialize)]
struct ByNumberQuery {
    profile_id: i64,
    /// Canal que se está viendo: si el número se repite entre proveedores, gana el suyo.
    near: Option<i64>,
}

async fn get_channel_by_number(
    State(state): State<TvState>,
    AxumPath(number): AxumPath<i64>,
    Query(query): Query<ByNumberQuery>,
) -> Result<Json<ChannelSummary>, TvError> {
    let channel = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::channel_by_number(conn, number, query.near, include_adult)
    })
    .await?;
    channel.map(|c| Json(c.into())).ok_or(TvError::NotFound)
}

#[derive(Deserialize)]
struct RecentQuery {
    profile_id: i64,
    limit: Option<i64>,
}

/// Últimos canales vistos por este perfil, el más reciente primero.
async fn get_recent_channels(State(state): State<TvState>, Query(query): Query<RecentQuery>) -> Result<Json<Vec<ChannelSummary>>, TvError> {
    let limit = query.limit.unwrap_or(12).clamp(1, 50);
    let channels = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::recent_channels(conn, query.profile_id, include_adult, limit)
    })
    .await?;
    Ok(Json(channels.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct EpgSearchQuery {
    profile_id: i64,
    q: String,
    limit: Option<i64>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct EpgSearchResult {
    pub channel: ChannelSummary,
    pub title: String,
    pub subtitle: Option<String>,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
}

/// Búsqueda en la guía (F13): lo que se emite ahora o más adelante cuyo título contiene
/// `q`, por orden de comienzo.
async fn get_epg_search(State(state): State<TvState>, Query(query): Query<EpgSearchQuery>) -> Result<Json<Vec<EpgSearchResult>>, TvError> {
    let q = query.q.trim().to_string();
    if q.chars().count() < 2 {
        return Err(TvError::BadRequest("escribe al menos dos letras".to_string()));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let now = now_epoch();
    let results = with_db(state.db_path, move |conn| {
        let include_adult = reads::profile_allows_adult(conn, query.profile_id)?;
        reads::epg_search(conn, &q, now, include_adult, limit)
    })
    .await?;
    Ok(Json(
        results
            .into_iter()
            .map(|r| EpgSearchResult { channel: r.channel.into(), title: r.title, subtitle: r.subtitle, start_ts: r.start_ts, stop_ts: r.stop_ts })
            .collect(),
    ))
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ReminderResponse {
    pub channel: ChannelSummary,
    pub title: String,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
}

/// Recordatorios del perfil que aún no han terminado, el más próximo primero. Los ya
/// terminados se borran al pedirlos: no sirven de nada.
async fn get_reminders(State(state): State<TvState>, Query(query): Query<ProfileQuery>) -> Result<Json<Vec<ReminderResponse>>, TvError> {
    let now = now_epoch();
    let reminders = with_db(state.db_path, move |conn| reads::list_reminders(conn, query.profile_id, now)).await?;
    Ok(Json(
        reminders
            .into_iter()
            .map(|r| ReminderResponse { channel: r.channel.into(), title: r.title, start_ts: r.start_ts, stop_ts: r.stop_ts })
            .collect(),
    ))
}

#[derive(Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ReminderInput {
    #[ts(type = "number")]
    pub profile_id: i64,
    #[ts(type = "number")]
    pub channel_id: i64,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub stop_ts: i64,
    pub title: String,
}

async fn post_reminder(State(state): State<TvState>, Json(input): Json<ReminderInput>) -> Result<StatusCode, TvError> {
    if input.stop_ts <= now_epoch() {
        return Err(TvError::BadRequest("ese programa ya ha terminado".to_string()));
    }
    with_db(state.db_path, move |conn| reads::add_reminder(conn, input.profile_id, input.channel_id, input.start_ts, input.stop_ts, &input.title))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ReminderKey {
    profile_id: i64,
    channel_id: i64,
    start_ts: i64,
}

async fn delete_reminder(State(state): State<TvState>, Query(key): Query<ReminderKey>) -> Result<StatusCode, TvError> {
    with_db(state.db_path, move |conn| reads::delete_reminder(conn, key.profile_id, key.channel_id, key.start_ts)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{self, ProviderKind, TitleFields};
    use crate::db::schema::open_connection_at;
    use crate::sources::model::{MediaKind, RawCategory, RawChannel};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;

    struct Seeded {
        db: PathBuf,
        kid: i64,
        adult: i64,
        ids: HashMap<&'static str, i64>,
    }

    /// Canales con números desordenados respecto a sus ids (como tras una
    /// resincronización que añade canales en medio), uno de adultos y uno sin número.
    fn seed() -> Seeded {
        use rand::RngExt;
        let suffix: u64 = rand::rng().random();
        let db = std::env::temp_dir().join(format!("vibe-iptv-tv-test-{}-{suffix:x}.sqlite3", std::process::id()));
        let conn = open_connection_at(&db).unwrap();
        let kid = queries::insert_profile(&conn, "Peque", None, false).unwrap();
        let adult = queries::insert_profile(&conn, "Adulto", None, true).unwrap();
        let provider = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let categories = vec![
            RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "General".into() },
            RawCategory { remote_id: "9".into(), kind: MediaKind::Live, name: "XXX Adultos".into() },
        ];
        let map = queries::upsert_categories(&conn, provider, MediaKind::Live, &categories).unwrap();
        let mut ids = HashMap::new();
        for (name, remote, category, number, epg) in [
            ("Tres", "a", "1", Some(3), Some("tres.es")),
            ("Uno", "b", "1", Some(1), Some("uno.es")),
            ("Sin número", "c", "1", None, None),
            ("Dos", "d", "1", Some(2), None),
            ("Picante", "e", "9", Some(4), Some("xxx.es")),
        ] {
            let channel = RawChannel {
                remote_id: remote.into(),
                category_remote_id: Some(category.into()),
                raw_name: name.into(),
                logo_url: None,
                epg_channel_id: epg.map(Into::into),
                has_catchup: false,
                catchup_days: 0,
                sort_order: number,
            };
            let fields = TitleFields { clean_title: name, year: None, match_key: "n/a", quality: None, language: None };
            queries::upsert_channel(&conn, provider, &map, &channel, &fields).unwrap();
            ids.insert(name, conn.query_row("SELECT id FROM channels WHERE remote_id = ?1", [remote], |r| r.get(0)).unwrap());
        }
        Seeded { db, kid, adult, ids }
    }

    async fn get_json(db: &PathBuf, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = tv_router(TvState { db_path: db.clone() }).oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn a_channel_is_found_by_its_provider_number_and_adult_ones_only_for_adults() {
        let s = seed();
        let (status, json) = get_json(&s.db, &format!("/api/live/channels/by-number/2?profile_id={}", s.kid)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["display_name"], "Dos");
        assert_eq!(json["number"], 2);
        assert_eq!(get_json(&s.db, &format!("/api/live/channels/by-number/4?profile_id={}", s.kid)).await.0, StatusCode::NOT_FOUND);
        assert_eq!(get_json(&s.db, &format!("/api/live/channels/by-number/4?profile_id={}", s.adult)).await.0, StatusCode::OK);
        assert_eq!(get_json(&s.db, &format!("/api/live/channels/by-number/99?profile_id={}", s.adult)).await.0, StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(&s.db);
    }

    #[tokio::test]
    async fn a_repeated_number_resolves_to_the_provider_being_watched() {
        let s = seed();
        let other = {
            let conn = open_connection_at(&s.db).unwrap();
            let provider = queries::insert_provider(&conn, ProviderKind::Xtream, "Otro", "http://z", None, 1, None).unwrap();
            let channel = RawChannel {
                remote_id: "z2".into(),
                category_remote_id: None,
                raw_name: "Dos del otro".into(),
                logo_url: None,
                epg_channel_id: None,
                has_catchup: false,
                catchup_days: 0,
                sort_order: Some(2),
            };
            let fields = TitleFields { clean_title: "Dos del otro", year: None, match_key: "n/a", quality: None, language: None };
            queries::upsert_channel(&conn, provider, &HashMap::new(), &channel, &fields).unwrap();
            let id: i64 = conn.query_row("SELECT id FROM channels WHERE remote_id = 'z2'", [], |r| r.get(0)).unwrap();
            id
        };
        let (_, from_first) = get_json(&s.db, &format!("/api/live/channels/by-number/2?profile_id={}&near={}", s.kid, s.ids["Uno"])).await;
        assert_eq!(from_first["display_name"], "Dos");
        let (_, from_other) = get_json(&s.db, &format!("/api/live/channels/by-number/2?profile_id={}&near={other}", s.kid)).await;
        assert_eq!(from_other["display_name"], "Dos del otro");
        let _ = std::fs::remove_file(&s.db);
    }

    #[tokio::test]
    async fn recent_channels_come_newest_first() {
        let s = seed();
        {
            let conn = open_connection_at(&s.db).unwrap();
            for (name, played_at) in [("Uno", 100), ("Tres", 300), ("Dos", 200), ("Picante", 400)] {
                conn.execute(
                    "INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, finished, played_at) VALUES (?1, 'channel', ?2, 0, 0, ?3)",
                    rusqlite::params![s.kid, s.ids[name], played_at],
                )
                .unwrap();
            }
        }
        let (_, json) = get_json(&s.db, &format!("/api/live/recent?profile_id={}", s.kid)).await;
        let names: Vec<&str> = json.as_array().unwrap().iter().map(|c| c["display_name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Tres", "Dos", "Uno"], "el de adultos no sale para un perfil infantil");
        let _ = std::fs::remove_file(&s.db);
    }

    #[tokio::test]
    async fn the_guide_search_finds_current_and_future_programmes_only() {
        let s = seed();
        let now = now_epoch();
        {
            let conn = open_connection_at(&s.db).unwrap();
            for (channel, start, stop, title) in [
                ("uno.es", now - 7200, now - 3600, "Fútbol de ayer"),
                ("uno.es", now - 600, now + 600, "Fútbol en directo"),
                ("tres.es", now + 3600, now + 7200, "Más fútbol"),
                ("xxx.es", now + 100, now + 200, "Fútbol picante"),
                ("tres.es", now + 100, now + 200, "Noticias"),
            ] {
                conn.execute(
                    "INSERT INTO epg_programmes (channel_id, start_ts, stop_ts, title) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![channel, start, stop, title],
                )
                .unwrap();
            }
        }
        let (status, json) = get_json(&s.db, &format!("/api/epg/search?profile_id={}&q=f%C3%BAtbol", s.kid)).await;
        assert_eq!(status, StatusCode::OK);
        let titles: Vec<&str> = json.as_array().unwrap().iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Fútbol en directo", "Más fútbol"]);
        assert_eq!(json[0]["channel"]["display_name"], "Uno");
        assert_eq!(get_json(&s.db, &format!("/api/epg/search?profile_id={}&q=f", s.kid)).await.0, StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&s.db);
    }

    #[tokio::test]
    async fn reminders_are_listed_soonest_first_and_finished_ones_disappear() {
        let s = seed();
        let now = now_epoch();
        let router = || tv_router(TvState { db_path: s.db.clone() });
        let post = |channel: i64, start: i64, stop: i64, title: &str| {
            let body = serde_json::json!({ "profile_id": s.kid, "channel_id": channel, "start_ts": start, "stop_ts": stop, "title": title });
            Request::builder().method("POST").uri("/api/reminders").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
        };
        assert_eq!(router().oneshot(post(s.ids["Tres"], now + 3600, now + 7200, "Más tarde")).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(router().oneshot(post(s.ids["Uno"], now + 60, now + 600, "Enseguida")).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert_eq!(router().oneshot(post(s.ids["Dos"], now - 600, now - 60, "Ya pasó")).await.unwrap().status(), StatusCode::BAD_REQUEST);

        let (_, json) = get_json(&s.db, &format!("/api/reminders?profile_id={}", s.kid)).await;
        let titles: Vec<&str> = json.as_array().unwrap().iter().map(|r| r["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Enseguida", "Más tarde"]);

        {
            // Uno que terminó mientras tanto se borra al pedir la lista.
            let conn = open_connection_at(&s.db).unwrap();
            conn.execute("UPDATE reminders SET stop_ts = ?1 WHERE title = 'Enseguida'", [now - 1]).unwrap();
        }
        let delete = Request::builder()
            .method("DELETE")
            .uri(format!("/api/reminders?profile_id={}&channel_id={}&start_ts={}", s.kid, s.ids["Tres"], now + 3600))
            .body(Body::empty())
            .unwrap();
        assert_eq!(router().oneshot(delete).await.unwrap().status(), StatusCode::NO_CONTENT);
        let (_, json) = get_json(&s.db, &format!("/api/reminders?profile_id={}", s.kid)).await;
        assert_eq!(json.as_array().unwrap().len(), 0);
        let _ = std::fs::remove_file(&s.db);
    }

    /// Paginar por el orden real: antes se pedía la página siguiente por id ordenando por
    /// número, y con números desordenados respecto a los ids se saltaban canales.
    #[test]
    fn channel_pages_follow_the_number_order_without_gaps_or_repeats() {
        let s = seed();
        let conn = open_connection_at(&s.db).unwrap();
        let mut seen = Vec::new();
        let mut after = None;
        loop {
            let page = reads::list_channels_page(&conn, reads::ChannelScope::default(), None, after, 2, true).unwrap();
            if page.is_empty() {
                break;
            }
            let last = page.last().unwrap();
            after = Some((last.number, last.id));
            seen.extend(page.iter().map(|c| c.display_name.clone()));
        }
        assert_eq!(seen, ["Uno", "Dos", "Tres", "Picante", "Sin número"]);
        let _ = std::fs::remove_file(&s.db);
    }
}
