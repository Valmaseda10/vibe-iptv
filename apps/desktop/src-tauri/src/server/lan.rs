//! Modo LAN (F10, ver `docs/DECISIONS.md` para el porqué del diseño): el servidor sigue
//! el mismo socket de siempre (regla R2 — un solo puerto efímero en `127.0.0.1`, que
//! también es alcanzable desde la LAN si el sistema operativo enruta hacia él), pero
//! toda petición que no venga de loopback pasa primero por [`require_local_or_paired`],
//! un middleware que la rechaza salvo que el modo LAN esté activado y lleve un token de
//! dispositivo emparejado válido. Sin eso, la app se comporta exactamente igual que
//! antes de F10: local únicamente.
//!
//! El "StreamGuard distribuido" que pide `PROMPTS.md` no necesita código aparte: todo
//! dispositivo, local o LAN, pasa por el mismo `server::stream` y el mismo
//! `StreamGuard` de este proceso — es "distribuido" en el sentido de que el PC decide
//! por todos, no en el de coordinar varios procesos.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Path as AxumPath, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::db::{queries, reads, schema, DbError};
use crate::util::pairing::{self, PairingState};

const LAN_ENABLED_SETTING_KEY: &str = "lan_enabled";
/// Único endpoint que un dispositivo sin emparejar todavía puede llamar: es justo cómo
/// consigue su primer token, así que no puede exigírsele uno de antemano.
const PAIR_REDEEM_PATH: &str = "/api/lan/pair";

#[derive(Clone)]
pub struct LanState {
    pub db_path: PathBuf,
    /// En memoria, reflejo del ajuste persistido (`app_settings.lan_enabled`) — se lee
    /// en cada petición sin tocar la base de datos; `post_enable`/`post_disable`
    /// actualizan las dos cosas a la vez.
    pub enabled: Arc<AtomicBool>,
    pub pairing: Arc<PairingState>,
}

pub fn lan_router(state: LanState) -> Router {
    Router::new()
        .route("/api/lan/status", get(get_status))
        .route("/api/lan/enable", post(post_enable))
        .route("/api/lan/disable", post(post_disable))
        .route("/api/lan/pairing/start", post(post_pairing_start))
        .route(PAIR_REDEEM_PATH, post(post_pair))
        .route("/api/lan/devices/:id/revoke", post(post_revoke_device))
        .with_state(state)
}

#[derive(Debug, thiserror::Error)]
enum LanError {
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for LanError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, LanError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_connection_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| LanError::Internal)?
    .map_err(LanError::Db)
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PairedDeviceItem {
    #[ts(type = "number")]
    pub id: i64,
    pub device_name: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub last_seen_at: Option<i64>,
    pub revoked: bool,
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct LanStatusResponse {
    pub enabled: bool,
    pub devices: Vec<PairedDeviceItem>,
    /// IP de este PC en la red local, para enseñarla junto al código de emparejamiento
    /// (el otro dispositivo la necesita para saber a quién conectarse; no hay ningún
    /// mecanismo de descubrimiento tipo mDNS todavía). `None` si no se pudo determinar
    /// (sin red, por ejemplo) — no es un error, solo no hay nada que enseñar.
    pub local_address: Option<String>,
}

/// Trata de averiguar con qué IP se ve este PC desde la red local, sin abrir ninguna
/// conexión de verdad: un socket UDP "conectado" a una IP pública solo hace que el
/// sistema operativo resuelva la ruta de salida y la interfaz asociada, no manda ningún
/// paquete (UDP no tiene apretón de manos). Trueco estándar, sin depender de ninguna
/// librería nueva solo para esto.
fn detect_local_lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip().to_string())
}

async fn get_status(State(state): State<LanState>) -> Result<Json<LanStatusResponse>, LanError> {
    let devices = with_db(state.db_path.clone(), |conn| reads::list_paired_devices(conn)).await?;
    Ok(Json(LanStatusResponse {
        enabled: state.enabled.load(Ordering::SeqCst),
        devices: devices
            .into_iter()
            .map(|d| PairedDeviceItem {
                id: d.id,
                device_name: d.device_name,
                created_at: d.created_at,
                last_seen_at: d.last_seen_at,
                revoked: d.revoked_at.is_some(),
            })
            .collect(),
        local_address: detect_local_lan_address(),
    }))
}

async fn post_enable(State(state): State<LanState>) -> Result<StatusCode, LanError> {
    with_db(state.db_path.clone(), |conn| queries::set_setting(conn, LAN_ENABLED_SETTING_KEY, "1")).await?;
    state.enabled.store(true, Ordering::SeqCst);
    Ok(StatusCode::NO_CONTENT)
}

async fn post_disable(State(state): State<LanState>) -> Result<StatusCode, LanError> {
    with_db(state.db_path.clone(), |conn| queries::set_setting(conn, LAN_ENABLED_SETTING_KEY, "0")).await?;
    state.enabled.store(false, Ordering::SeqCst);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PairingStartResponse {
    pub code: String,
    /// Segundos desde ahora hasta que caduca — más cómodo para el frontend que mandar
    /// un instante que no significa nada fuera de este proceso (`std::time::Instant` no
    /// es una fecha de calendario). `u64` -> "number", no "bigint": un TTL de unos
    /// minutos cabe sobrado en `Number.MAX_SAFE_INTEGER` (igual que los `i64` de esta
    /// API, ver `commands::ProviderSummary`).
    #[ts(type = "number")]
    pub expires_in_s: u64,
}

/// Se llama desde el PC (el dispositivo que ya tiene la app), para enseñar un código en
/// pantalla. No necesita el modo LAN activado para generarlo — pero de nada sirve si
/// luego el otro dispositivo no puede canjearlo porque el modo LAN está apagado
/// (`post_pair` sí lo exige).
async fn post_pairing_start(State(state): State<LanState>) -> Json<PairingStartResponse> {
    let (code, expires_at) = state.pairing.start();
    let expires_in_s = expires_at.saturating_duration_since(std::time::Instant::now()).as_secs();
    Json(PairingStartResponse { code, expires_in_s })
}

#[derive(Deserialize)]
struct PairInput {
    code: String,
    device_name: String,
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PairResponse {
    #[ts(type = "number")]
    pub device_id: i64,
    /// Solo se manda esta vez: no se guarda en claro en ningún sitio (regla R5, mismo
    /// criterio que una credencial de proveedor).
    pub token: String,
}

#[derive(Debug, thiserror::Error)]
enum PairError {
    #[error("El modo LAN no está activado en este PC.")]
    LanDisabled,
    #[error("Código incorrecto o caducado. Pide uno nuevo desde el PC.")]
    InvalidCode,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for PairError {
    fn into_response(self) -> Response {
        let status = match &self {
            PairError::LanDisabled => StatusCode::FORBIDDEN,
            PairError::InvalidCode => StatusCode::UNAUTHORIZED,
            PairError::Db(_) | PairError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

/// Lo llama el dispositivo NUEVO, sin token todavía — es como consigue el primero. Por
/// eso el middleware ([`require_local_or_paired`]) deja pasar esta ruta concreta sin
/// pedir `Authorization`, aunque sí exige que el modo LAN esté activado.
async fn post_pair(State(state): State<LanState>, Json(input): Json<PairInput>) -> Result<Json<PairResponse>, PairError> {
    if !state.enabled.load(Ordering::SeqCst) {
        return Err(PairError::LanDisabled);
    }
    if !state.pairing.redeem(&input.code) {
        return Err(PairError::InvalidCode);
    }

    let token = pairing::generate_device_token();
    let token_hash = pairing::hash_token(&token);
    let device_name = input.device_name;
    let device_id = tokio::task::spawn_blocking({
        let db_path = state.db_path.clone();
        move || -> crate::db::Result<i64> {
            let conn = schema::open_connection_at(&db_path)?;
            queries::insert_paired_device(&conn, &device_name, &token_hash)
        }
    })
    .await
    .map_err(|_| PairError::Internal)?
    .map_err(PairError::Db)?;

    Ok(Json(PairResponse { device_id, token }))
}

async fn post_revoke_device(State(state): State<LanState>, AxumPath(device_id): AxumPath<i64>) -> Result<StatusCode, LanError> {
    with_db(state.db_path.clone(), move |conn| queries::revoke_paired_device(conn, device_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Puerta de entrada del modo LAN: se aplica sobre el router completo (`server::mod`),
/// no solo sobre `lan_router`, así que cubre catálogo y streaming por igual. Sin
/// `ConnectInfo` (solo pasa en pruebas con `.oneshot()`, que no pasan por un socket TCP
/// real) se trata como local — el servidor de verdad siempre lo lleva, ver
/// `server::serve`.
pub async fn require_local_or_paired(
    State(state): State<LanState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let is_loopback = connect_info.map(|ConnectInfo(addr)| addr.ip().is_loopback()).unwrap_or(true);
    if is_loopback {
        return next.run(request).await;
    }

    if !state.enabled.load(Ordering::SeqCst) {
        return (StatusCode::FORBIDDEN, "El modo LAN no está activado en este PC.").into_response();
    }

    if request.uri().path() == PAIR_REDEEM_PATH {
        return next.run(request).await;
    }

    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "Este dispositivo no está emparejado.").into_response();
    };

    let token_hash = pairing::hash_token(token);
    let db_path = state.db_path.clone();
    let device_id = tokio::task::spawn_blocking(move || -> crate::db::Result<Option<i64>> {
        let conn = schema::open_readonly_at(&db_path)?;
        reads::find_active_device_by_token_hash(&conn, &token_hash)
    })
    .await;

    let Ok(Ok(Some(device_id))) = device_id else {
        return (StatusCode::UNAUTHORIZED, "Token de dispositivo inválido o revocado.").into_response();
    };

    // Cortesía informativa para Ajustes ("visto por última vez..."); no bloquea la
    // petición real ni la hace fallar si escribir esto fallara por lo que sea.
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        if let Ok(conn) = schema::open_connection_at(&db_path) {
            let _ = queries::touch_paired_device_last_seen(&conn, device_id);
        }
    });

    next.run(request).await
}

/// Lee el ajuste persistido al arrancar (ver `server::serve`); si no hay ninguno
/// guardado todavía, el modo LAN empieza desactivado (mismo criterio que
/// `ParentalGuard`: por defecto, lo más restrictivo).
pub fn read_lan_enabled_setting(conn: &rusqlite::Connection) -> crate::db::Result<bool> {
    Ok(reads::get_setting(conn, LAN_ENABLED_SETTING_KEY)?.as_deref() == Some("1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body as AxumBody;
    use axum::http::Request as AxumRequest;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::db::schema::open_connection_at;

    fn temp_db_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vibe-iptv-lan-test-{label}-{}-{}.sqlite3", std::process::id(), rand_suffix()))
    }

    /// Aleatorio de verdad, no la hora: dos tests en paralelo podían sacar el mismo
    /// nanosegundo, abrir el mismo fichero SQLite y fallar de forma intermitente.
    fn rand_suffix() -> u64 {
        use rand::RngExt;
        rand::rng().random()
    }

    fn base_state(db_path: std::path::PathBuf) -> LanState {
        LanState { db_path, enabled: Arc::new(AtomicBool::new(false)), pairing: PairingState::new() }
    }

    #[tokio::test]
    async fn status_starts_disabled_with_no_devices() {
        let db_path = temp_db_path("status");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());

        let response = lan_router(state)
            .oneshot(AxumRequest::builder().uri("/api/lan/status").body(AxumBody::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["enabled"], false);
        assert_eq!(parsed["devices"].as_array().unwrap().len(), 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn pairing_end_to_end_start_pair_and_status_reflects_it() {
        let db_path = temp_db_path("pair-ok");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());
        state.enabled.store(true, Ordering::SeqCst);
        let router = lan_router(state.clone());

        let start_response = router
            .clone()
            .oneshot(AxumRequest::builder().method("POST").uri("/api/lan/pairing/start").body(AxumBody::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::OK);
        let body = start_response.into_body().collect().await.unwrap().to_bytes();
        let start: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let code = start["code"].as_str().unwrap();

        let pair_body = serde_json::json!({ "code": code, "device_name": "Fire TV de prueba" }).to_string();
        let pair_response = router
            .clone()
            .oneshot(
                AxumRequest::builder()
                    .method("POST")
                    .uri("/api/lan/pair")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(AxumBody::from(pair_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pair_response.status(), StatusCode::OK);
        let body = pair_response.into_body().collect().await.unwrap().to_bytes();
        let pair: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(pair["token"].as_str().unwrap().len() >= 32, "el token debería ser largo y aleatorio");

        let status_response = router
            .oneshot(AxumRequest::builder().uri("/api/lan/status").body(AxumBody::empty()).unwrap())
            .await
            .unwrap();
        let body = status_response.into_body().collect().await.unwrap().to_bytes();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status["devices"][0]["device_name"], "Fire TV de prueba");
        assert_eq!(status["devices"][0]["revoked"], false);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn pair_fails_when_lan_is_disabled() {
        let db_path = temp_db_path("pair-disabled");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone()); // enabled = false por defecto

        let pair_body = serde_json::json!({ "code": "123456", "device_name": "x" }).to_string();
        let response = lan_router(state)
            .oneshot(
                AxumRequest::builder()
                    .method("POST")
                    .uri("/api/lan/pair")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(AxumBody::from(pair_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn pair_fails_with_a_wrong_code() {
        let db_path = temp_db_path("pair-wrong");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());
        state.enabled.store(true, Ordering::SeqCst);
        let (code, _) = state.pairing.start();
        let wrong = if code == "000000" { "111111" } else { "000000" };

        let pair_body = serde_json::json!({ "code": wrong, "device_name": "x" }).to_string();
        let response = lan_router(state)
            .oneshot(
                AxumRequest::builder()
                    .method("POST")
                    .uri("/api/lan/pair")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(AxumBody::from(pair_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn revoking_a_device_marks_it_revoked_in_status() {
        let db_path = temp_db_path("revoke");
        let conn = open_connection_at(&db_path).unwrap();
        let device_id = queries::insert_paired_device(&conn, "Dispositivo", "hash-de-prueba").unwrap();
        drop(conn);
        let state = base_state(db_path.clone());
        let router = lan_router(state);

        let response = router
            .clone()
            .oneshot(
                AxumRequest::builder()
                    .method("POST")
                    .uri(format!("/api/lan/devices/{device_id}/revoke"))
                    .body(AxumBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let status_response =
            router.oneshot(AxumRequest::builder().uri("/api/lan/status").body(AxumBody::empty()).unwrap()).await.unwrap();
        let body = status_response.into_body().collect().await.unwrap().to_bytes();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status["devices"][0]["revoked"], true);

        let _ = std::fs::remove_file(&db_path);
    }

    /// El middleware se prueba aparte de `lan_router` porque necesita envolver una ruta
    /// cualquiera "protegida" — aquí una de mentira, ya que probarlo contra el router
    /// real de streaming/catálogo es cosa de `server::mod` (donde de verdad se monta).
    fn protected_router(state: LanState) -> Router {
        async fn ok_handler() -> &'static str {
            "ok"
        }
        Router::new()
            .route("/protegido", get(ok_handler))
            .route_layer(axum::middleware::from_fn_with_state(state.clone(), require_local_or_paired))
            .with_state(state)
    }

    fn request_from(addr: &str, uri: &str) -> AxumRequest<AxumBody> {
        let mut request = AxumRequest::builder().uri(uri).body(AxumBody::empty()).unwrap();
        request.extensions_mut().insert(ConnectInfo(addr.parse::<SocketAddr>().unwrap()));
        request
    }

    #[tokio::test]
    async fn loopback_requests_always_pass_regardless_of_lan_settings() {
        let db_path = temp_db_path("mw-loopback");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone()); // LAN desactivado

        let response = protected_router(state).oneshot(request_from("127.0.0.1:54321", "/protegido")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn non_loopback_requests_are_rejected_when_lan_is_disabled() {
        let db_path = temp_db_path("mw-lan-off");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());

        let response = protected_router(state).oneshot(request_from("192.168.1.50:1234", "/protegido")).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn non_loopback_requests_without_a_token_are_rejected_when_lan_is_enabled() {
        let db_path = temp_db_path("mw-no-token");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());
        state.enabled.store(true, Ordering::SeqCst);

        let response = protected_router(state).oneshot(request_from("192.168.1.50:1234", "/protegido")).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn non_loopback_requests_with_a_valid_token_pass_and_a_revoked_one_is_rejected() {
        let db_path = temp_db_path("mw-token");
        let conn = open_connection_at(&db_path).unwrap();
        let token = pairing::generate_device_token();
        let device_id = queries::insert_paired_device(&conn, "Fire TV", &pairing::hash_token(&token)).unwrap();
        drop(conn);
        let state = base_state(db_path.clone());
        state.enabled.store(true, Ordering::SeqCst);
        let router = protected_router(state.clone());

        let mut request = request_from("192.168.1.50:1234", "/protegido");
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Revocado: el mismo token deja de valer inmediatamente.
        let conn = open_connection_at(&db_path).unwrap();
        queries::revoke_paired_device(&conn, device_id).unwrap();
        drop(conn);
        let mut request = request_from("192.168.1.50:1234", "/protegido");
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn the_pair_endpoint_itself_never_needs_a_token() {
        let db_path = temp_db_path("mw-pair-open");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone());
        state.enabled.store(true, Ordering::SeqCst);
        let (code, _) = state.pairing.start();

        async fn ok_handler() -> &'static str {
            "ok"
        }
        let router = Router::new()
            .route(PAIR_REDEEM_PATH, get(ok_handler)) // GET de mentira solo para probar que el middleware deja pasar la ruta
            .route_layer(axum::middleware::from_fn_with_state(state.clone(), require_local_or_paired))
            .with_state(state.clone());

        let response = router.oneshot(request_from("192.168.1.50:1234", PAIR_REDEEM_PATH)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = code; // el código no se usa aquí: solo se comprueba que la ruta no exige token.

        let _ = std::fs::remove_file(&db_path);
    }
}
