//! Perfiles estilo Netflix: crear, listar, editar, borrar, y "entrar" (comprobar el PIN
//! de ese perfil concreto, si tiene uno). Sustituye al control parental global de F8 —
//! ver `util::parental` y `docs/DECISIONS.md`.
//!
//! Todavía no sincroniza contra Supabase (eso es el siguiente tramo, ver
//! `sync::supabase`): esto es la base local, funciona igual con o sin cuenta.

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::db::{queries, reads, schema, DbError};
use crate::util::parental::{hash_pin, verify_pin};

#[derive(Clone)]
pub struct ProfilesState {
    pub db_path: PathBuf,
}

pub fn profiles_router(state: ProfilesState) -> Router {
    Router::new()
        .route("/api/profiles", get(get_profiles).post(post_create_profile))
        .route("/api/profiles/:id", patch(patch_profile).delete(delete_profile_route))
        .route("/api/profiles/:id/pin", post(post_set_pin))
        .route("/api/profiles/:id/enter", post(post_enter_profile))
        .with_state(state)
}

#[derive(Debug, thiserror::Error)]
enum ProfilesError {
    #[error("Perfil no encontrado.")]
    NotFound,
    #[error("PIN incorrecto.")]
    WrongPin,
    #[error("No se puede borrar el único perfil que queda.")]
    LastProfile,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for ProfilesError {
    fn into_response(self) -> Response {
        let status = match &self {
            ProfilesError::NotFound => StatusCode::NOT_FOUND,
            ProfilesError::WrongPin => StatusCode::UNAUTHORIZED,
            ProfilesError::LastProfile => StatusCode::BAD_REQUEST,
            ProfilesError::Db(_) | ProfilesError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, ProfilesError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_connection_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| ProfilesError::Internal)?
    .map_err(ProfilesError::Db)
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ProfileSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub avatar: Option<String>,
    pub has_pin: bool,
    pub adult_ok: bool,
}

impl From<reads::ProfileRow> for ProfileSummary {
    fn from(row: reads::ProfileRow) -> Self {
        Self { id: row.id, name: row.name, avatar: row.avatar, has_pin: row.has_pin, adult_ok: row.adult_ok }
    }
}

async fn get_profiles(State(state): State<ProfilesState>) -> Result<Json<Vec<ProfileSummary>>, ProfilesError> {
    let profiles = with_db(state.db_path, |conn| reads::list_profiles(conn)).await?;
    Ok(Json(profiles.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ProfileInput {
    pub name: String,
    pub avatar: Option<String>,
    pub adult_ok: bool,
}

async fn post_create_profile(State(state): State<ProfilesState>, Json(input): Json<ProfileInput>) -> Result<Json<ProfileSummary>, ProfilesError> {
    let profile_id = with_db(state.db_path.clone(), move |conn| {
        queries::insert_profile(conn, &input.name, input.avatar.as_deref(), input.adult_ok)
    })
    .await?;
    let profile = with_db(state.db_path, move |conn| reads::get_profile(conn, profile_id)).await?.ok_or(ProfilesError::NotFound)?;
    Ok(Json(profile.into()))
}

async fn patch_profile(
    State(state): State<ProfilesState>,
    AxumPath(profile_id): AxumPath<i64>,
    Json(input): Json<ProfileInput>,
) -> Result<StatusCode, ProfilesError> {
    with_db(state.db_path, move |conn| queries::update_profile(conn, profile_id, &input.name, input.avatar.as_deref(), input.adult_ok)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Se niega a borrar el último perfil que queda: la app siempre necesita al menos uno
/// para tener sentido — es la misma razón por la que Netflix tampoco te deja quedarte
/// sin ninguno.
async fn delete_profile_route(State(state): State<ProfilesState>, AxumPath(profile_id): AxumPath<i64>) -> Result<StatusCode, ProfilesError> {
    let count = with_db(state.db_path.clone(), |conn| reads::count_profiles(conn)).await?;
    if count <= 1 {
        return Err(ProfilesError::LastProfile);
    }
    with_db(state.db_path, move |conn| queries::delete_profile(conn, profile_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SetProfilePinInput {
    /// `None` quita el PIN de este perfil (vuelve a entrarse sin pedir nada).
    pub pin: Option<String>,
}

/// Cambiar/quitar el PIN de un perfil no pide el PIN actual (a diferencia del viejo
/// control parental global de F8): esto es "administrar perfiles" desde dentro de la
/// propia app, con acceso físico ya asumido — igual de simple que renombrar un perfil.
async fn post_set_pin(
    State(state): State<ProfilesState>,
    AxumPath(profile_id): AxumPath<i64>,
    Json(input): Json<SetProfilePinInput>,
) -> Result<StatusCode, ProfilesError> {
    let pin_hash = input.pin.as_deref().map(hash_pin);
    with_db(state.db_path, move |conn| queries::set_profile_pin_hash(conn, profile_id, pin_hash.as_deref())).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct EnterProfileInput {
    pin: Option<String>,
}

/// Se llama al elegir un perfil en el selector. Sin PIN configurado, cualquier `pin`
/// (incluido `null`) entra. Con PIN, hace falta que coincida — 401 sin distinguir "sin
/// PIN" de "PIN incorrecto" en el código de estado (aunque aquí el mensaje sí lo
/// distingue: a diferencia del login de una cuenta, no hay nada que enumerar atacando
/// esto — los perfiles ya son visibles en el selector antes de entrar).
async fn post_enter_profile(
    State(state): State<ProfilesState>,
    AxumPath(profile_id): AxumPath<i64>,
    Json(input): Json<EnterProfileInput>,
) -> Result<Json<ProfileSummary>, ProfilesError> {
    let db_path = state.db_path.clone();
    let (profile, pin_hash) = with_db(db_path, move |conn| {
        let profile = reads::get_profile(conn, profile_id)?;
        let pin_hash = reads::get_profile_pin_hash(conn, profile_id)?;
        Ok((profile, pin_hash))
    })
    .await?;
    let profile = profile.ok_or(ProfilesError::NotFound)?;

    if let Some(stored_hash) = pin_hash {
        match input.pin {
            Some(pin) if verify_pin(&pin, &stored_hash) => {}
            _ => return Err(ProfilesError::WrongPin),
        }
    }

    Ok(Json(profile.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::db::schema::open_connection_at;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vibe-iptv-profiles-test-{label}-{}-{}.sqlite3", std::process::id(), rand_suffix()))
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    fn router_for(db_path: PathBuf) -> Router {
        profiles_router(ProfilesState { db_path })
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn post_json(router: &Router, uri: &str, body: serde_json::Value) -> Response {
        router
            .clone()
            .oneshot(Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_list_and_update_a_profile() {
        let db_path = temp_db_path("crud");
        open_connection_at(&db_path).unwrap();
        let router = router_for(db_path.clone());

        let created = post_json(&router, "/api/profiles", serde_json::json!({"name": "Jorge", "avatar": "rojo", "adult_ok": true})).await;
        assert_eq!(created.status(), StatusCode::OK);
        let created_json = body_json(created).await;
        let profile_id = created_json["id"].as_i64().unwrap();
        assert_eq!(created_json["has_pin"], false);

        let list = router.clone().oneshot(Request::builder().uri("/api/profiles").body(Body::empty()).unwrap()).await.unwrap();
        let list_json = body_json(list).await;
        assert_eq!(list_json.as_array().unwrap().len(), 1);
        assert_eq!(list_json[0]["name"], "Jorge");

        let updated = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/profiles/{profile_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"name": "Jorge V.", "avatar": "azul", "adult_ok": false}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::NO_CONTENT);

        let list = router.oneshot(Request::builder().uri("/api/profiles").body(Body::empty()).unwrap()).await.unwrap();
        let list_json = body_json(list).await;
        assert_eq!(list_json[0]["name"], "Jorge V.");
        assert_eq!(list_json[0]["adult_ok"], false);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn cannot_delete_the_last_remaining_profile() {
        let db_path = temp_db_path("last");
        let conn = open_connection_at(&db_path).unwrap();
        let profile_id = queries::insert_profile(&conn, "Único", None, false).unwrap();
        drop(conn);
        let router = router_for(db_path.clone());

        let response = router.oneshot(Request::builder().method("DELETE").uri(format!("/api/profiles/{profile_id}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn deleting_one_of_several_profiles_succeeds() {
        let db_path = temp_db_path("delete-ok");
        let conn = open_connection_at(&db_path).unwrap();
        let first = queries::insert_profile(&conn, "Uno", None, false).unwrap();
        queries::insert_profile(&conn, "Dos", None, false).unwrap();
        drop(conn);
        let router = router_for(db_path.clone());

        let response = router.oneshot(Request::builder().method("DELETE").uri(format!("/api/profiles/{first}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn entering_a_profile_without_a_pin_never_fails() {
        let db_path = temp_db_path("enter-no-pin");
        let conn = open_connection_at(&db_path).unwrap();
        let profile_id = queries::insert_profile(&conn, "Sin PIN", None, false).unwrap();
        drop(conn);
        let router = router_for(db_path.clone());

        let response = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": null})).await;
        assert_eq!(response.status(), StatusCode::OK);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn entering_a_profile_with_a_pin_requires_the_right_one() {
        let db_path = temp_db_path("enter-pin");
        let conn = open_connection_at(&db_path).unwrap();
        let profile_id = queries::insert_profile(&conn, "Con PIN", None, false).unwrap();
        queries::set_profile_pin_hash(&conn, profile_id, Some(&hash_pin("1234"))).unwrap();
        drop(conn);
        let router = router_for(db_path.clone());

        let wrong = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": "0000"})).await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let missing = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": null})).await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let right = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": "1234"})).await;
        assert_eq!(right.status(), StatusCode::OK);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn setting_and_clearing_a_pin_roundtrip() {
        let db_path = temp_db_path("set-pin");
        let conn = open_connection_at(&db_path).unwrap();
        let profile_id = queries::insert_profile(&conn, "P", None, false).unwrap();
        drop(conn);
        let router = router_for(db_path.clone());

        let response = post_json(&router, &format!("/api/profiles/{profile_id}/pin"), serde_json::json!({"pin": "5555"})).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let entered = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": "5555"})).await;
        assert_eq!(entered.status(), StatusCode::OK);

        let response = post_json(&router, &format!("/api/profiles/{profile_id}/pin"), serde_json::json!({"pin": null})).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let entered = post_json(&router, &format!("/api/profiles/{profile_id}/enter"), serde_json::json!({"pin": null})).await;
        assert_eq!(entered.status(), StatusCode::OK);

        let _ = std::fs::remove_file(&db_path);
    }
}
