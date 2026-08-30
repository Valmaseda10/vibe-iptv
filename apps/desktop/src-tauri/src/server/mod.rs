pub mod routes;

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;

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
pub async fn serve(listener: StdTcpListener, image_cache_dir: PathBuf) -> std::io::Result<()> {
    // En Windows, un socket duplicado con `try_clone()` no hereda el modo no bloqueante
    // del original (es un atributo de Winsock por handle, no por socket subyacente).
    // Sin esto, `accept()` bloquea el hilo del runtime y el servidor deja de responder
    // sin ningún error visible.
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, routes::build_router(image_cache_dir)).await
}
