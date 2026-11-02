use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
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
        return image_response(&image_id, bytes);
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
        // Primero a un temporal y luego se renombra: una petición simultánea de la misma
        // imagen nunca lee un fichero a medio escribir.
        let partial = cache_dir.join(format!("{size}_{image_id}.{}.part", std::process::id()));
        if tokio::fs::write(&partial, &bytes).await.is_ok() && tokio::fs::rename(&partial, &cache_path).await.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
    }

    image_response(&image_id, bytes.to_vec())
}

/// El logo de un canal, el que encuentre `catalog::logos` (ya guardado en el aparato). 404
/// si no hay ninguno que funcione: la interfaz enseña las iniciales.
async fn serve_logo(State((db_path, dir)): State<(PathBuf, PathBuf)>, AxumPath(channel_id): AxumPath<i64>) -> Response {
    let Some(path) = crate::catalog::logos::resolve(&db_path, &dir, channel_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            // Un día: si el logo cambia (otra fuente, uno elegido a mano), se ve al día siguiente
            // o al reabrir la pantalla.
            ([(header::CONTENT_TYPE, content_type_for(name)), (header::CACHE_CONTROL, "public, max-age=86400")], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Las imágenes de TMDB no cambian nunca para un mismo nombre: la WebView puede guardarlas
/// sin volver a preguntar (las listas virtualizadas montan y desmontan portadas sin parar).
fn image_response(image_id: &str, bytes: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, content_type_for(image_id)), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], bytes).into_response()
}

/// Tope de la caché de imágenes en disco: por encima, se borran las más antiguas hasta
/// quedar en `IMAGE_CACHE_TARGET`. Un catálogo de 100.000 títulos (R4) llenaría gigas.
const IMAGE_CACHE_LIMIT: u64 = 600 * 1024 * 1024;
const IMAGE_CACHE_TARGET: u64 = 450 * 1024 * 1024;

/// Recorta la caché de imágenes al arrancar (y borra temporales sueltos). Devuelve cuántos
/// ficheros ha quitado.
pub fn trim_image_cache(dir: &std::path::Path) -> usize {
    trim_image_cache_to(dir, IMAGE_CACHE_LIMIT, IMAGE_CACHE_TARGET)
}

fn trim_image_cache_to(dir: &std::path::Path, limit: u64, target: u64) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "part") {
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        total += meta.len();
        files.push((meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH), meta.len(), path));
    }
    if total <= limit {
        return removed;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, len, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

fn content_type_for(filename: &str) -> &'static str {
    match filename.rsplit('.').next().unwrap_or_default() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

/// Un parón del directo visto por el reproductor del PC (`useLiveCushion`).
#[derive(Deserialize)]
struct StallReport {
    channel_id: i64,
    /// Vídeo que el reproductor tenía guardado por delante al pararse.
    ahead_s: f64,
    current_time_s: f64,
    /// `grabacion` si ya se veía desde la grabación (hls.js); si no, la salida rápida.
    #[serde(default)]
    mode: Option<String>,
}

/// Apunta en el registro un parón del directo, para «Exportar diagnóstico»: dónde estaba,
/// cuánto vídeo quedaba guardado y desde dónde se veía. Sin nada del proveedor (R5).
async fn log_stall(Json(report): Json<StallReport>) -> StatusCode {
    tracing::info!(
        channel_id = report.channel_id,
        ahead_s = report.ahead_s,
        current_time_s = report.current_time_s,
        mode = report.mode.as_deref().unwrap_or("directo"),
        "directo: parón en el reproductor"
    );
    StatusCode::NO_CONTENT
}

/// Construye el router del catálogo y de salud/imágenes. `/stream/*` y
/// `/api/playback/tier/*` viven en `server::stream` (F6) y se añaden aparte en
/// `server::serve`, que es quien aplica el CORS al router final ya completo — antes
/// vivía aquí y solo cubría este router, dejando sin CORS las rutas de F6 (un `fetch()`
/// del WebView a `/api/playback/tier/*` fallaba en silencio; `<video src>` no lo notó
/// porque cargar un recurso de vídeo no pasa por CORS, solo `fetch`/`XHR`).
pub fn build_router(image_cache_dir: PathBuf, db_path: PathBuf) -> Router {
    // Los logos de canal se guardan junto a la caché de portadas (`<datos>/logos`).
    let logos_dir = image_cache_dir.parent().map(|p| p.join("logos")).unwrap_or_else(|| image_cache_dir.join("logos"));
    let logos = Router::new().route("/logo/:channel_id", get(serve_logo)).with_state((db_path.clone(), logos_dir));
    let health_and_images = Router::new()
        .route("/api/health", get(health))
        .route("/img/:size/:image_id", get(serve_image))
        .route("/api/diag/stall", post(log_stall))
        .with_state(image_cache_dir);

    health_and_images.merge(logos).merge(super::api::catalog_router(db_path))
}

#[cfg(test)]
mod tests {
    use super::{build_router, trim_image_cache_to};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
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

    #[test]
    fn image_cache_trim_removes_the_oldest_until_under_target() {
        let dir = std::env::temp_dir().join(format!("vibe-iptv-img-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, name) in ["w500_a.jpg", "w500_b.jpg", "w500_c.jpg", "w500_d.jpg"].iter().enumerate() {
            let path = dir.join(name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
        std::fs::write(dir.join("w500_x.jpg.1.part"), b"half").unwrap();
        // 400 bytes con tope 350: se borran las dos más viejas hasta quedar en 200.
        assert_eq!(trim_image_cache_to(&dir, 350, 200), 3);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, vec!["w500_c.jpg", "w500_d.jpg"]);
        // Por debajo del tope no se toca nada.
        assert_eq!(trim_image_cache_to(&dir, 350, 200), 0);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(response.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"fake-jpeg-bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
