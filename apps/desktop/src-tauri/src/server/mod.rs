pub mod api;
pub mod routes;
pub mod stream;

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::Arc;

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

    let router = routes::build_router(image_cache_dir, db_path).merge(stream::stream_router(stream_state));
    axum::serve(listener, router).await
}
