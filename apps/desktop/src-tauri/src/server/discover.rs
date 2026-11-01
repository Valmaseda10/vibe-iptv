//! Sagas y fichas de personas (F17, parte 2): `GET /api/collections/:id` y
//! `GET /api/people/:id`. Lo que hay en el catálogo sale al instante; lo que hay que pedir
//! a TMDB (las partes de la saga que faltan, la biografía) se pide aquí la primera vez,
//! con un límite de espera, y se guarda.

use std::path::PathBuf;
use std::time::Duration;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::tmdb::{TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
use crate::catalog::{collections, people};
use crate::db::{now_epoch, reads, schema, DbError};
use crate::util::keychain;

/// Lo más que se espera a TMDB antes de responder con lo que ya hay.
const TMDB_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct DiscoverState {
    pub db_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum DiscoverError {
    #[error("no encontrado")]
    NotFound,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for DiscoverError {
    fn into_response(self) -> Response {
        let status = match self {
            DiscoverError::NotFound => StatusCode::NOT_FOUND,
            DiscoverError::Db(_) | DiscoverError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub fn discover_router(state: DiscoverState) -> Router {
    Router::new()
        .route("/api/collections/:id", get(get_collection))
        .route("/api/people/:id", get(get_person))
        .with_state(state)
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, DiscoverError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| DiscoverError::Internal)?
    .map_err(DiscoverError::Db)
}

fn tmdb_client() -> Option<TmdbClient> {
    keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).ok().map(TmdbClient::new)
}

#[derive(Deserialize)]
struct ProfileQuery {
    profile_id: i64,
}

/// Una película de una saga, en su orden.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SagaEntryResponse {
    pub title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    /// `null` si no está en el catálogo (se enseña en gris).
    #[ts(type = "number | null")]
    pub movie_id: Option<i64>,
    pub watched: bool,
    pub progress: Option<f64>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct CollectionResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub entries: Vec<SagaEntryResponse>,
    /// La que toca ver («Seguir con…» o «Empezar»).
    #[ts(type = "number | null")]
    pub next_movie_id: Option<i64>,
    #[ts(type = "number")]
    pub available: i64,
    #[ts(type = "number")]
    pub watched: i64,
}

async fn get_collection(State(state): State<DiscoverState>, AxumPath(id): AxumPath<i64>, Query(query): Query<ProfileQuery>) -> Result<Json<CollectionResponse>, DiscoverError> {
    let db_path = state.db_path;
    let header = with_db(db_path.clone(), move |conn| collections::header(conn, id)).await?.ok_or(DiscoverError::NotFound)?;
    // Las partes que faltan (y el orden de verdad) son de TMDB: la primera vez, o al mes.
    if collections::parts_stale(&header, now_epoch()) {
        if let Some(client) = tmdb_client() {
            match tokio::time::timeout(TMDB_WAIT, client.collection(id)).await {
                Ok(Ok(detail)) => {
                    if let Err(err) = with_db(db_path.clone(), move |conn| collections::store_detail(conn, &detail)).await {
                        tracing::warn!(error = %err, "no se pudo guardar la saga");
                    }
                }
                Ok(Err(err)) => tracing::warn!(collection_id = id, error = %err, "no se pudo pedir la saga a TMDB"),
                Err(_) => tracing::info!(collection_id = id, "TMDB tarda: la saga se enseña con lo que hay"),
            }
        }
    }
    let profile_id = query.profile_id;
    let response = with_db(db_path, move |conn| {
        let access = reads::profile_access(conn, profile_id)?;
        let header = collections::header(conn, id)?;
        let Some(header) = header else { return Ok(None) };
        let entries = collections::entries(conn, id, profile_id, access, header.parts.as_deref())?;
        let next_movie_id = collections::next_to_watch(&entries, None);
        let available = entries.iter().filter(|e| e.movie_id.is_some()).count() as i64;
        let watched = entries.iter().filter(|e| e.movie_id.is_some() && e.watched).count() as i64;
        Ok(Some(CollectionResponse {
            id: header.id,
            name: header.name,
            overview: header.overview,
            poster_path: header.poster_path,
            backdrop_path: header.backdrop_path,
            entries: entries
                .into_iter()
                .map(|e| SagaEntryResponse { title: e.title, year: e.year, poster_path: e.poster_path, movie_id: e.movie_id, watched: e.watched, progress: e.progress })
                .collect(),
            next_movie_id,
            available,
            watched,
        }))
    })
    .await?
    .ok_or(DiscoverError::NotFound)?;
    if response.available == 0 {
        // Nada de la saga que este perfil pueda ver: como si no existiera.
        return Err(DiscoverError::NotFound);
    }
    Ok(Json(response))
}

/// Un título en la ficha de una persona.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PersonTitleResponse {
    /// `movie` o `series`.
    pub kind: String,
    #[ts(type = "number")]
    pub id: i64,
    pub clean_title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    /// El personaje, o «Dirección» / «Creación».
    pub role: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PersonResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub profile_path: Option<String>,
    /// `Acting`, `Directing`… (de TMDB).
    pub department: Option<String>,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    pub titles: Vec<PersonTitleResponse>,
}

async fn get_person(State(state): State<DiscoverState>, AxumPath(id): AxumPath<i64>, Query(query): Query<ProfileQuery>) -> Result<Json<PersonResponse>, DiscoverError> {
    let db_path = state.db_path;
    let mut detail = with_db(db_path.clone(), move |conn| people::cached_person_detail(conn, id)).await?;
    if detail.is_none() {
        if let Some(client) = tmdb_client() {
            match tokio::time::timeout(TMDB_WAIT, client.person(id)).await {
                Ok(Ok(fetched)) => {
                    let stored = fetched.clone();
                    if let Err(err) = with_db(db_path.clone(), move |conn| people::store_person_detail(conn, id, &stored)).await {
                        tracing::warn!(error = %err, "no se pudo guardar la biografía");
                    }
                    detail = Some(fetched);
                }
                Ok(Err(err)) => tracing::warn!(person_id = id, error = %err, "no se pudo pedir la persona a TMDB"),
                Err(_) => tracing::info!(person_id = id, "TMDB tarda: la ficha sale sin biografía"),
            }
        }
    }
    let detail = detail.unwrap_or_default();
    let profile_id = query.profile_id;
    let response = with_db(db_path, move |conn| {
        let Some(person) = people::person(conn, id)? else { return Ok(None) };
        let access = reads::profile_access(conn, profile_id)?;
        let titles = people::person_titles(conn, id, access)?;
        let mut out = Vec::with_capacity(titles.len());
        for kind in ["movie", "series"] {
            let ids: Vec<i64> = titles.iter().filter(|t| t.item_kind == kind).map(|t| t.item_id).collect();
            let cards = reads::browse_cards(conn, kind, &ids)?;
            for title in titles.iter().filter(|t| t.item_kind == kind) {
                let Some(card) = cards.get(&title.item_id) else { continue };
                let role = match title.role.as_str() {
                    "director" => Some("Dirección".to_string()),
                    "creator" => Some("Creación".to_string()),
                    _ => title.character.clone(),
                };
                out.push((titles.iter().position(|t| t.item_kind == kind && t.item_id == title.item_id).unwrap_or(usize::MAX), PersonTitleResponse {
                    kind: kind.to_string(),
                    id: title.item_id,
                    clean_title: card.clean_title.clone(),
                    year: card.year,
                    poster_path: card.poster_path.clone(),
                    role,
                }));
            }
        }
        out.sort_by_key(|(order, _)| *order);
        Ok(Some(PersonResponse {
            id: person.id,
            name: person.name,
            profile_path: person.profile_path,
            department: person.department,
            biography: detail.biography,
            birthday: detail.birthday,
            deathday: detail.deathday,
            place_of_birth: detail.place_of_birth,
            titles: out.into_iter().map(|(_, title)| title).collect(),
        }))
    })
    .await?
    .ok_or(DiscoverError::NotFound)?;
    if response.titles.is_empty() {
        return Err(DiscoverError::NotFound);
    }
    Ok(Json(response))
}
