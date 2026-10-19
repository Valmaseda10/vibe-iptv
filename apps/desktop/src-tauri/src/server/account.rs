//! Cuenta en la nube: registro, inicio y cierre de sesión, recuperación y cambio de
//! contraseña (F11: Firebase, antes Supabase). Ver `sync` para qué viaja y qué no —
//! catálogo, credenciales de proveedor y streaming siguen siendo exclusivos de este PC
//! (reglas R2/R5 de `CLAUDE.md` intactas).
//!
//! Local-first: el estado de la cuenta se responde sin tocar la red, y un fallo
//! pasajero de la nube responde 503 para que la interfaz pueda ofrecer entrar sin
//! conexión en vez de dejar al usuario fuera de su propio catálogo.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::sync::backend::{CloudBackend, CloudError, Session};
use crate::sync::{engine, session};

/// Lo que se espera a que se envíe lo pendiente antes de cerrar sesión.
const LOGOUT_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AccountState {
    pub db_path: PathBuf,
    pub cloud: Arc<dyn CloudBackend>,
}

pub fn account_router(state: AccountState) -> Router {
    Router::new()
        .route("/api/account/status", get(get_status))
        .route("/api/account/signup", post(post_signup))
        .route("/api/account/login", post(post_login))
        .route("/api/account/logout", post(post_logout))
        .route("/api/account/forgot-password", post(post_forgot_password))
        .route("/api/account/reset-password", post(post_reset_password))
        .route("/api/account/change-password", post(post_change_password))
        .with_state(state)
}

/// Cómo va la sincronización con la nube, para Ajustes.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct CloudSyncStatus {
    /// Cambios hechos en este PC que todavía no han llegado a la nube.
    #[ts(type = "number")]
    pub pending_changes: i64,
    /// Epoch de la última vuelta completa sin errores, si la hubo desde que arrancó la app.
    #[ts(type = "number | null")]
    pub last_synced_at: Option<i64>,
    /// Último error, en español, si la última vuelta falló.
    pub last_error: Option<String>,
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct AccountStatusResponse {
    pub logged_in: bool,
    pub email: Option<String>,
    pub sync: Option<CloudSyncStatus>,
}

/// Sin red de por medio: hay sesión si hay una guardada. Si la nube la revoca, el motor
/// de sincronización la borra al intentar renovarla y la siguiente consulta ya lo ve.
async fn get_status(State(state): State<AccountState>) -> Json<AccountStatusResponse> {
    let Some(session) = session::load() else {
        return Json(AccountStatusResponse { logged_in: false, email: None, sync: None });
    };
    let status = engine::status();
    Json(AccountStatusResponse {
        logged_in: true,
        email: Some(session.email),
        sync: Some(CloudSyncStatus {
            pending_changes: engine::pending_changes(&state.db_path).await,
            last_synced_at: status.last_synced_at,
            last_error: status.last_error,
        }),
    })
}

impl IntoResponse for CloudError {
    fn into_response(self) -> Response {
        let status = if self.is_transient() { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::UNAUTHORIZED };
        (status, self.to_string()).into_response()
    }
}

#[derive(Deserialize)]
struct CredentialsInput {
    email: String,
    password: String,
}

async fn signed_in(db_path: &Path, session: Session) -> Json<AccountStatusResponse> {
    let email = session.email.clone();
    // Antes de que la app enseñe nada: si los perfiles de este PC eran de otra cuenta,
    // fuera. Si fallara, la fusión lo vuelve a intentar antes de subir nada.
    if let Err(err) = engine::claim_local_data(db_path, &session.user_id).await {
        tracing::warn!(error = %err, "no se pudo comprobar de quién son los perfiles locales");
    }
    session::save(&session);
    // La fusión con lo que haya en la nube la hace el motor en segundo plano: el login
    // responde en cuanto la nube acepta las credenciales.
    engine::wake();
    Json(AccountStatusResponse { logged_in: true, email: Some(email), sync: None })
}

async fn post_signup(State(state): State<AccountState>, Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, CloudError> {
    let session = state.cloud.sign_up(input.email.trim(), &input.password).await?;
    Ok(signed_in(&state.db_path, session).await)
}

async fn post_login(State(state): State<AccountState>, Json(input): Json<CredentialsInput>) -> Result<Json<AccountStatusResponse>, CloudError> {
    let session = state.cloud.sign_in(input.email.trim(), &input.password).await?;
    Ok(signed_in(&state.db_path, session).await)
}

/// Cierra sesión localmente siempre. Antes intenta un último envío de lo pendiente,
/// con un tope de tiempo: sin red no debe quedarse colgado, y lo no enviado sigue en
/// este PC y se subirá en la fusión del próximo inicio de sesión.
async fn post_logout(State(state): State<AccountState>) -> StatusCode {
    let cloud = state.cloud.clone();
    let db_path = state.db_path.clone();
    let _ = tokio::time::timeout(LOGOUT_FLUSH_TIMEOUT, async move {
        if let Some(session) = session::get_valid(cloud.as_ref()).await {
            let _ = engine::flush_outbox(&db_path, cloud.as_ref(), &session).await;
        }
    })
    .await;
    if let Err(err) = engine::forget_account(&state.db_path).await {
        tracing::warn!(error = %err, "no se pudo limpiar el estado de sincronización al cerrar sesión");
    }
    session::clear();
    StatusCode::NO_CONTENT
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ForgotPasswordInput {
    pub email: String,
}

async fn post_forgot_password(State(state): State<AccountState>, Json(input): Json<ForgotPasswordInput>) -> Result<StatusCode, CloudError> {
    state.cloud.send_password_reset(input.email.trim()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ResetPasswordInput {
    pub email: String,
    /// El enlace completo del correo, o solo su código (`oobCode`).
    pub token_or_link: String,
    pub new_password: String,
}

/// Completa la recuperación y deja al usuario dentro: un cambio de contraseña con éxito
/// no debería obligar a iniciar sesión otra vez aparte.
async fn post_reset_password(State(state): State<AccountState>, Json(input): Json<ResetPasswordInput>) -> Result<Json<AccountStatusResponse>, CloudError> {
    state.cloud.confirm_password_reset(&input.token_or_link, &input.new_password).await?;
    let session = state.cloud.sign_in(input.email.trim(), &input.new_password).await?;
    Ok(signed_in(&state.db_path, session).await)
}

#[derive(Deserialize)]
struct ChangePasswordInput {
    current_password: String,
    new_password: String,
}

/// Cambiar contraseña con la sesión ya abierta (Ajustes). Se vuelve a pedir la actual
/// aunque haya sesión: quien encuentre el PC con la sesión abierta no debería poder
/// cambiarla sin saberla. Ese mismo inicio de sesión da un token recién emitido, que es
/// lo que Firebase exige para cambiar la contraseña.
async fn post_change_password(State(state): State<AccountState>, Json(input): Json<ChangePasswordInput>) -> Result<StatusCode, CloudError> {
    let current = session::load().ok_or_else(|| CloudError::Rejected("No hay ninguna sesión iniciada.".to_string()))?;
    let verified = state.cloud.sign_in(&current.email, &input.current_password).await?;
    let renewed = state.cloud.change_password(&verified, &input.new_password).await?;
    session::save(&renewed);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::memory::MemoryBackend;
    use crate::util::keychain::TEST_LOCK as TEST_KEYCHAIN_LOCK;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn router(cloud: Arc<MemoryBackend>) -> Router {
        account_router(AccountState { db_path: temp_db(), cloud })
    }

    fn temp_db() -> PathBuf {
        use rand::RngExt;
        let suffix: u64 = rand::rng().random();
        let path = std::env::temp_dir().join(format!("vibe-iptv-account-test-{}-{suffix:x}.sqlite3", std::process::id()));
        crate::db::schema::open_connection_at(&path).unwrap();
        path
    }

    async fn post(router: Router, uri: &str, body: &str) -> (StatusCode, String) {
        let response = router
            .oneshot(Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn status_reports_logged_out_when_no_session_saved() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        session::clear();
        let response = router(Arc::new(MemoryBackend::new()))
            .oneshot(Request::builder().uri("/api/account/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["logged_in"], false);
    }

    /// El fallo real que motivó F11: con la nube caída, la app no puede saber si la
    /// sesión sigue valiendo, y no por eso debe echar al usuario.
    #[tokio::test]
    async fn status_keeps_the_user_signed_in_while_the_cloud_is_down() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let cloud = Arc::new(MemoryBackend::new());
        session::save(&Session { expires_at: 0, ..cloud.session() });
        cloud.fail_everything(Some(CloudError::Unavailable));

        let response = router(cloud).oneshot(Request::builder().uri("/api/account/status").body(Body::empty()).unwrap()).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["logged_in"], true);
        assert_eq!(parsed["sync"]["pending_changes"], 0);
        session::clear();
    }

    #[tokio::test]
    async fn login_failing_for_an_outage_is_a_503_not_a_wrong_password() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let cloud = Arc::new(MemoryBackend::new());
        cloud.fail_everything(Some(CloudError::Network));
        let (status, _) = post(router(cloud), "/api/account/login", r#"{"email":"a@b.c","password":"x"}"#).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn login_saves_the_session() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        session::clear();
        let (status, body) = post(router(Arc::new(MemoryBackend::new())), "/api/account/login", r#"{"email":"qa@example.com","password":"x"}"#).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(session::load().unwrap().user_id, "uid-test");
        session::clear();
    }

    #[tokio::test]
    async fn logout_clears_the_session_even_without_network() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let cloud = Arc::new(MemoryBackend::new());
        session::save(&cloud.session());
        cloud.fail_everything(Some(CloudError::Network));
        let (status, _) = post(router(cloud), "/api/account/logout", "{}").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(session::load().is_none());
    }

    #[tokio::test]
    async fn change_password_rejects_when_there_is_no_session_at_all() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        session::clear();
        let (status, _) = post(router(Arc::new(MemoryBackend::new())), "/api/account/change-password", r#"{"current_password":"x","new_password":"y"}"#).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
