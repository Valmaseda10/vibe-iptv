//! Cuenta de sincronización (F "perfiles Netflix"): login/registro contra Supabase.
//! Ver `sync::supabase` para el porqué de un cliente propio y qué NO pasa nunca por
//! aquí — catálogo, credenciales de proveedor y streaming siguen siendo cosa exclusiva
//! de este PC (reglas R2/R5 de `CLAUDE.md` intactas).

use std::path::PathBuf;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::sync::supabase::{SupabaseClient, SupabaseError};
use crate::sync::{engine, session};

#[derive(Clone)]
pub struct AccountState {
    pub db_path: PathBuf,
}

pub fn account_router(state: AccountState) -> Router {
    Router::new()
        .route("/api/account/status", get(get_status))
        .route("/api/account/signup", post(post_signup))
        .route("/api/account/login", post(post_login))
        .route("/api/account/logout", post(post_logout))
        .route("/api/account/forgot-password", post(post_forgot_password))
        .route("/api/account/reset-password", post(post_reset_password))
        .with_state(state)
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

async fn post_signup(State(state): State<AccountState>, Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, SupabaseError> {
    let session = SupabaseClient::new().sign_up(&input.email, &input.password).await?;
    let email = session.email.clone();
    session::save(&session);
    tokio::spawn(engine::pull_and_merge(state.db_path));
    Ok(Json(AccountStatusResponse { logged_in: true, email: Some(email) }))
}

async fn post_login(State(state): State<AccountState>, Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, SupabaseError> {
    let session = SupabaseClient::new().sign_in(&input.email, &input.password).await?;
    let email = session.email.clone();
    session::save(&session);
    // En segundo plano: el login responde en cuanto Supabase confirma las credenciales,
    // no hace falta esperar a que baje y fusione perfiles/favoritos/progreso.
    tokio::spawn(engine::pull_and_merge(state.db_path));
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

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ForgotPasswordInput {
    pub email: String,
}

async fn post_forgot_password(Json(input): Json<ForgotPasswordInput>) -> Result<StatusCode, SupabaseError> {
    SupabaseClient::new().recover(&input.email).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ResetPasswordInput {
    pub email: String,
    /// El enlace completo del correo, o solo el valor de su parámetro `token=` — la app
    /// no tiene página web propia que lo reciba, así que el usuario lo pega a mano.
    pub token_or_link: String,
    pub new_password: String,
}

/// Extrae el token de un enlace de recuperación pegado entero, o lo deja tal cual si ya
/// es solo el token — para no obligar al usuario a saber cuál de los dos ha copiado.
fn extract_recovery_token(token_or_link: &str) -> &str {
    let trimmed = token_or_link.trim();
    match trimmed.find("token=") {
        Some(pos) => {
            let after = &trimmed[pos + "token=".len()..];
            after.split('&').next().unwrap_or(after)
        }
        None => trimmed,
    }
}

/// Completa la recuperación: valida el enlace/código contra Supabase, fija la
/// contraseña nueva con la sesión que eso devuelve, y deja al usuario logueado — un
/// cambio de contraseña con éxito no debería obligar a iniciar sesión otra vez aparte.
async fn post_reset_password(State(state): State<AccountState>, Json(input): Json<ResetPasswordInput>) -> Result<Json<AccountStatusResponse>, SupabaseError> {
    let token = extract_recovery_token(&input.token_or_link);
    let client = SupabaseClient::new();
    let session = client.verify_recovery(&input.email, token).await?;
    client.update_password(&session.access_token, &input.new_password).await?;
    let email = session.email.clone();
    session::save(&session);
    tokio::spawn(engine::pull_and_merge(state.db_path));
    Ok(Json(AccountStatusResponse { logged_in: true, email: Some(email) }))
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
        let response = account_router(AccountState { db_path: std::env::temp_dir() })
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

        let response = account_router(AccountState { db_path: std::env::temp_dir() })
            .oneshot(Request::builder().method("POST").uri("/api/account/logout").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(session::load().is_none());
    }

    #[test]
    fn extract_recovery_token_accepts_a_bare_token() {
        assert_eq!(extract_recovery_token("abc123"), "abc123");
    }

    #[test]
    fn extract_recovery_token_pulls_it_out_of_a_pasted_link() {
        let link = "https://beqentvogqamkopsnfos.supabase.co/auth/v1/verify?token=abc123&type=recovery&redirect_to=http://localhost:3000";
        assert_eq!(extract_recovery_token(link), "abc123");
    }

    #[test]
    fn extract_recovery_token_trims_surrounding_whitespace() {
        assert_eq!(extract_recovery_token("  abc123  "), "abc123");
    }
}
