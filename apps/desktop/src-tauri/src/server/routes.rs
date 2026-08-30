use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
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

/// Sirve `poster_path`/`backdrop_path` de TMDB desde caché en disco, descargando si
/// falta. `size` es un token de tamaño de TMDB (`w500`, `w1280`, ...); `image_id` es el
/// nombre de fichero tal cual lo da TMDB (sin la barra inicial). Paso 7 de
/// `docs/ARCHITECTURE.md`: cola de imágenes, en segundo plano, sin bloquear el catálogo.
async fn serve_image(State(cache_dir): State<PathBuf>, AxumPath((size, image_id)): AxumPath<(String, String)>) -> Response {
    if size.contains(['.', '/', '\\']) || image_id.contains(['/', '\\']) || image_id.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let cache_path = cache_dir.join(format!("{size}_{image_id}"));

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return ([(header::CONTENT_TYPE, content_type_for(&image_id))], bytes).into_response();
    }

    let url = format!("https://image.tmdb.org/t/p/{size}/{image_id}");
    let response = match reqwest::get(&url).await {
        Ok(response) if response.status().is_success() => response,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    if tokio::fs::create_dir_all(&cache_dir).await.is_ok() {
        let _ = tokio::fs::write(&cache_path, &bytes).await;
    }

    ([(header::CONTENT_TYPE, content_type_for(&image_id))], bytes).into_response()
}

fn content_type_for(filename: &str) -> &'static str {
    if filename.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

/// Construye el router de la API local. `/stream/*` llega en F6.
///
/// CORS abierto a cualquier origen: el WebView de Tauri sirve el frontend desde un
/// origen distinto al del servidor (`http://localhost:1420` en dev, `tauri://localhost`
/// empaquetado), y este servidor solo escucha en 127.0.0.1 sin cookies ni sesión (R2).
/// Se revisará al abrir la LAN en la fase F10.
pub fn build_router(image_cache_dir: PathBuf, db_path: PathBuf) -> Router {
    let health_and_images = Router::new()
        .route("/api/health", get(health))
        .route("/img/:size/:image_id", get(serve_image))
        .with_state(image_cache_dir);

    health_and_images
        .merge(super::api::catalog_router(db_path))
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
        let response = build_router(std::env::temp_dir(), std::env::temp_dir())
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn image_route_rejects_path_traversal() {
        let response = build_router(std::env::temp_dir(), std::env::temp_dir())
            .oneshot(Request::builder().uri("/img/w500/..%2f..%2fetc").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn image_route_serves_from_cache_when_present() {
        let dir = std::env::temp_dir().join(format!("vibe-iptv-img-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("w500_abc.jpg"), b"fake-jpeg-bytes").unwrap();

        let response = build_router(dir.clone(), std::env::temp_dir())
            .oneshot(Request::builder().uri("/img/w500/abc.jpg").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"fake-jpeg-bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
