use axum::{routing::get, Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use ts_rs::TS;

/// Estado del núcleo devuelto por `GET /api/health`.
#[derive(Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct HealthStatus {
    pub status: String,
    pub version: String,
}

async fn health() -> Json<HealthStatus> {
    Json(HealthStatus {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Construye el router de la API local. `/stream/*` y `/img/*` llegan en fases
/// posteriores (ver `docs/ARCHITECTURE.md`).
///
/// CORS abierto a cualquier origen: el WebView de Tauri sirve el frontend desde un
/// origen distinto al del servidor (`http://localhost:1420` en dev, `tauri://localhost`
/// empaquetado), y este servidor solo escucha en 127.0.0.1 sin cookies ni sesión (R2).
/// Se revisará al abrir la LAN en la fase F10.
pub fn build_router() -> Router {
    Router::new()
        .route("/api/health", get(health))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_returns_ok_status() {
        let response = build_router()
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }
}
