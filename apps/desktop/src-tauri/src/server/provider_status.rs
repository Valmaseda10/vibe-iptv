//! ¿Responde ahora mismo la lista del proveedor? (F15). Inicio enseña un indicador discreto:
//! verde si responde, rojo con el motivo si no (el caso del usuario: fuera de España, sin la
//! VPN, el panel contesta "Country is not allowed").
//!
//! Se pregunta lo mínimo: la cuenta (Xtream) o el principio de la lista (M3U), y el
//! resultado se guarda unos minutos. Un panel bloquea a quien le pregunta demasiado
//! (ver `docs/DECISIONS.md`, F13).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::{queries, schema};
use crate::sources::xtream::{rejection_reason, XtreamSource, DEFAULT_USER_AGENT};
use crate::sources::Source;
use crate::util::keychain;

/// Lo que vale una comprobación antes de repetirla.
const CHECK_REUSE_FOR: Duration = Duration::from_secs(3 * 60);
/// Una comprobación no espera más que esto: el indicador no puede tardar en aparecer.
const CHECK_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub struct ProviderStatusState {
    pub db_path: PathBuf,
    cache: Arc<Mutex<HashMap<i64, (Instant, ProviderAvailability)>>>,
}

impl ProviderStatusState {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path, cache: Arc::default() }
    }
}

/// Si un proveedor responde ahora mismo, y si no, por qué.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ProviderAvailability {
    #[ts(type = "number")]
    pub provider_id: i64,
    pub name: String,
    pub available: bool,
    /// Motivo en palabras del usuario cuando no responde.
    pub message: Option<String>,
}

#[derive(Deserialize)]
struct StatusQuery {
    /// Repetir la comprobación aunque haya una reciente (botón "Reintentar").
    #[serde(default)]
    force: bool,
}

pub fn provider_status_router(state: ProviderStatusState) -> Router {
    Router::new().route("/api/providers/status", get(get_status)).with_state(state)
}

async fn get_status(State(state): State<ProviderStatusState>, Query(query): Query<StatusQuery>) -> Json<Vec<ProviderAvailability>> {
    let db_path = state.db_path.clone();
    let providers = tokio::task::spawn_blocking(move || -> crate::db::Result<Vec<queries::ProviderRow>> {
        let conn = schema::open_readonly_at(&db_path)?;
        queries::list_providers(&conn)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default();

    let mut result = Vec::new();
    for row in providers.into_iter().filter(|row| row.enabled) {
        let cached = state.cache.lock().ok().and_then(|cache| cache.get(&row.id).cloned());
        if let Some((checked_at, availability)) = cached {
            if !query.force && checked_at.elapsed() < CHECK_REUSE_FOR {
                result.push(availability);
                continue;
            }
        }
        let (id, name) = (row.id, row.name.clone());
        let outcome = tokio::time::timeout(CHECK_TIMEOUT, check(row)).await.unwrap_or_else(|_| {
            Err("El proveedor no responde (tarda demasiado). Puede estar caído o tu conexión no llega a él.".to_string())
        });
        let availability = ProviderAvailability { provider_id: id, name, available: outcome.is_ok(), message: outcome.err() };
        if let Ok(mut cache) = state.cache.lock() {
            cache.insert(id, (Instant::now(), availability.clone()));
        }
        result.push(availability);
    }
    Json(result)
}

async fn check(row: queries::ProviderRow) -> Result<(), String> {
    let credential_ref = row.credential_ref.clone().ok_or("Este proveedor no tiene credenciales guardadas.")?;
    let secret = keychain::read_secret(&credential_ref).map_err(|_| "No se pudieron leer las credenciales del proveedor.".to_string())?;
    match row.kind {
        queries::ProviderKind::Xtream => {
            let source = XtreamSource::new(row.base_url, row.username.unwrap_or_default(), secret, None).map_err(|err| err.to_string())?;
            let account = source.account_info().await.map_err(|err| err.to_string())?;
            if !account.auth_ok {
                return Err("El proveedor rechaza el usuario o la contraseña.".to_string());
            }
            if !account.status.is_empty() && !account.status.eq_ignore_ascii_case("active") {
                return Err(format!("La cuenta del proveedor no está activa (estado: {}).", account.status));
            }
            Ok(())
        }
        queries::ProviderKind::M3u => {
            // Solo el principio de la lista: basta para saber si responde y no descarga
            // decenas de megas.
            let client = reqwest::Client::builder().user_agent(DEFAULT_USER_AGENT).build().map_err(|err| err.to_string())?;
            let response = client
                .get(&secret)
                .header(reqwest::header::RANGE, "bytes=0-2047")
                .send()
                .await
                .map_err(|_| "No se pudo contactar con el proveedor.".to_string())?;
            let status = response.status();
            let body = response.bytes().await.unwrap_or_default();
            if !status.is_success() {
                return Err(rejection_reason(&body).unwrap_or_else(|| format!("El proveedor respondió {status}.")));
            }
            if !String::from_utf8_lossy(&body).trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U") {
                return Err("Lo que devuelve el proveedor no es una lista.".to_string());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::ProviderKind;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn panel(country_blocked: bool) -> String {
        let router = Router::new().route(
            "/player_api.php",
            get(move || async move {
                if country_blocked {
                    (StatusCode::FORBIDDEN, Json(serde_json::json!({"message": "Validation failed, Country is not allowed"})))
                } else {
                    (StatusCode::OK, Json(serde_json::json!({"user_info": {"auth": 1, "status": "Active", "max_connections": "1"}})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn reports_an_available_panel_and_a_country_blocked_one() {
        let _lock = keychain::TEST_LOCK.lock().await;
        let db_path = std::env::temp_dir().join(format!("vibe-iptv-provider-status-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&db_path);
        let (ok_url, blocked_url) = (panel(false).await, panel(true).await);
        let mut refs = Vec::new();
        {
            let conn = schema::open_connection_at(&db_path).unwrap();
            for (name, url) in [("Bueno", &ok_url), ("Bloqueado", &blocked_url)] {
                let id = queries::insert_provider(&conn, ProviderKind::Xtream, name, url, Some("u"), 1, None).unwrap();
                let credential_ref = format!("vibe-iptv-status-test-{}-{id}", std::process::id());
                keychain::store_secret(&credential_ref, "p").unwrap();
                queries::set_provider_credential_ref(&conn, id, &credential_ref).unwrap();
                refs.push(credential_ref);
            }
        }
        let router = provider_status_router(ProviderStatusState::new(db_path.clone()));
        let response = router.oneshot(Request::builder().uri("/api/providers/status").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let by_name = |name: &str| list.as_array().unwrap().iter().find(|p| p["name"] == name).unwrap().clone();
        assert_eq!(by_name("Bueno")["available"], true);
        assert_eq!(by_name("Bloqueado")["available"], false);
        assert!(by_name("Bloqueado")["message"].as_str().unwrap().contains("país"));

        for credential_ref in refs {
            let _ = keychain::delete_secret(&credential_ref);
        }
        let _ = std::fs::remove_file(&db_path);
    }
}
