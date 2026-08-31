//! Cuenta de sincronización (F "perfiles Netflix"): login/registro contra Supabase.
//! Ver `sync::supabase` para el porqué de un cliente propio y qué NO pasa nunca por
//! aquí — catálogo, credenciales de proveedor y streaming siguen siendo cosa exclusiva
//! de este PC (reglas R2/R5 de `CLAUDE.md` intactas).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::sync::session;
use crate::sync::supabase::{SupabaseClient, SupabaseError};

pub fn account_router() -> Router {
    Router::new()
        .route("/api/account/status", get(get_status))
        .route("/api/account/signup", post(post_signup))
        .route("/api/account/login", post(post_login))
        .route("/api/account/logout", post(post_logout))
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct AccountStatusResponse {
    pub logged_in: bool,
    pub email: Option<String>,
}

async fn get_status() -> Json<AccountStatusResponse> {
    match session::get_valid().await {
        Some(session) => Json(AccountStatusResponse { logged_in: true, email: Some(session.email) }),
        None => Json(AccountStatusResponse { logged_in: false, email: None }),
    }
}

#[derive(Deserialize)]
struct CredentialsInput {
    email: String,
    password: String,
}

impl IntoResponse for SupabaseError {
    fn into_response(self) -> Response {
        let status = match &self {
            SupabaseError::Network | SupabaseError::InvalidResponse => StatusCode::BAD_GATEWAY,
            SupabaseError::Api(_) => StatusCode::UNAUTHORIZED,
        };
        (status, self.to_string()).into_response()
    }
}

async fn post_signup(Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, SupabaseError> {
    let session = SupabaseClient::new().sign_up(&input.email, &input.password).await?;
    let email = session.email.clone();
    session::save(&session);
    Ok(Json(AccountStatusResponse { logged_in: true, email: Some(email) }))
}

async fn post_login(Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, SupabaseError> {
    let session = SupabaseClient::new().sign_in(&input.email, &input.password).await?;
    let email = session.email.clone();
    session::save(&session);
    Ok(Json(AccountStatusResponse { logged_in: true, email: Some(email) }))
}

/// Cierra sesión localmente siempre, aunque el aviso a Supabase falle (sin red, p.
/// ej.) — quedarse "atrapado" logueado porque no había conexión en ese instante sería
/// peor que el token quedar vivo en el servidor hasta que caduque solo.
async fn post_logout() -> StatusCode {
    if let Some(session) = session::load() {
        SupabaseClient::new().sign_out(&session.access_token).await;
    }
    session::clear();
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn status_reports_logged_out_when_no_session_saved() {
        session::clear(); // por si un test anterior de sync::session dejó algo.
        let response = account_router()
            .oneshot(Request::builder().uri("/api/account/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["logged_in"], false);
    }

    #[tokio::test]
    async fn logout_clears_a_locally_saved_session_even_without_network() {
        use crate::sync::supabase::Session;
        session::save(&Session {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            user_id: "u1".into(),
            email: "demo@example.com".into(),
            expires_at: crate::db::now_epoch() + 3600,
        });

        let response = account_router()
            .oneshot(Request::builder().method("POST").uri("/api/account/logout").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(session::load().is_none());
    }
}
