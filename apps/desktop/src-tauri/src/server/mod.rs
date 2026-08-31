pub mod api;
pub mod routes;
pub mod stream;

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::util::stream_guard::StreamGuard;

/// Puerto del servidor local, gestionado como estado de Tauri para que el frontend lo
/// consulte con el comando `get_server_port`.
pub struct ServerPort(pub u16);

/// Comando Tauri que expone al frontend el puerto efímero en el que escucha el
/// servidor local (regla R2 de CLAUDE.md: nunca un puerto fijo).
#[tauri::command]
pub fn get_server_port(port: tauri::State<ServerPort>) -> u16 {
    port.0
}

/// Sirve la API local sobre un socket ya reservado en `127.0.0.1`. El puerto se decide
/// de forma síncrona en el arranque, antes de que el frontend pueda pedirlo.
pub async fn serve(
    listener: StdTcpListener,
    image_cache_dir: PathBuf,
    db_path: PathBuf,
    stream_guard: Arc<StreamGuard>,
) -> std::io::Result<()> {
    // En Windows, un socket duplicado con `try_clone()` no hereda el modo no bloqueante
    // del original (es un atributo de Winsock por handle, no por socket subyacente).
    // Sin esto, `accept()` bloquea el hilo del runtime y el servidor deja de responder
    // sin ningún error visible.
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;

    let stream_state = stream::StreamState {
        db_path: db_path.clone(),
        stream_guard,
        // En v1 se busca en el PATH (ver docs/DECISIONS.md); F10 lo sustituye por el
        // sidecar empaquetado y esto deja de poder ser `None`.
        ffmpeg_path: crate::media::resolve_binary("ffmpeg"),
        ffprobe_path: crate::media::resolve_binary("ffprobe"),
    };
    if stream_state.ffmpeg_path.is_none() || stream_state.ffprobe_path.is_none() {
        tracing::warn!("ffmpeg y/o ffprobe no están en el PATH: el nivel 2 de reproducción no funcionará hasta F10");
    }

    let router = full_router(image_cache_dir, db_path, stream_state);
    axum::serve(listener, router).await
}

/// El router completo: catálogo + salud/imágenes + streaming, con CORS aplicado una
/// sola vez sobre el conjunto. Separado de `serve()` (que además reserva el socket real)
/// para poder probarlo con `oneshot` sin abrir ningún puerto.
///
/// CORS abierto a cualquier origen: el WebView de Tauri sirve el frontend desde un
/// origen distinto al del servidor (`http://localhost:1420` en dev, `tauri://localhost`
/// empaquetado), y este servidor solo escucha en 127.0.0.1 sin cookies ni sesión (R2).
/// Se revisará al abrir la LAN en la fase F10.
///
/// Tiene que envolver el router ya fusionado, no una de sus mitades por separado: un
/// `.layer()` aplicado antes de `.merge()` no cubre las rutas que se fusionan después.
/// Ese error real llegó a producción (F6): `fetch()` desde el WebView a
/// `/api/playback/tier/*` fallaba en silencio por CORS —`<video src>` no lo notó porque
/// cargar un recurso de vídeo no pasa por CORS, solo `fetch`/`XHR` sí— y el panel de
/// estadísticas del reproductor se quedó mostrando "nivel desconocido" para siempre.
fn full_router(image_cache_dir: PathBuf, db_path: PathBuf, stream_state: stream::StreamState) -> Router {
    routes::build_router(image_cache_dir, db_path)
        .merge(stream::stream_router(stream_state))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn cors_covers_routes_merged_after_the_catalog_router() {
        let stream_state = stream::StreamState {
            db_path: std::env::temp_dir(),
            stream_guard: StreamGuard::new(),
            ffmpeg_path: None,
            ffprobe_path: None,
        };
        let router = full_router(std::env::temp_dir(), std::env::temp_dir(), stream_state);

        // Una respuesta a una ruta de F6 (fusionada tras el CORS de F4) debe llevar la
        // cabecera igual que una del catálogo: el bug real fue que no la llevaba.
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/playback/tier/movie/999999")
                    .header(header::ORIGIN, "http://localhost:1420")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.headers().contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN), "falta la cabecera CORS");
    }
}
