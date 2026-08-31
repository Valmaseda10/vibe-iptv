// Previene la consola extra en Windows en builds de release; irrelevante en dev.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod commands;
mod db;
mod media;
mod server;
mod sources;
mod util;

use std::net::TcpListener as StdTcpListener;

use commands::{
    add_provider, export_diagnostics_log, has_tmdb_api_key, list_providers, match_tmdb, set_tmdb_api_key, sync_provider,
    validate_m3u, validate_xtream,
};
use server::{get_server_port, serve, ServerPort};
use tauri::Manager;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use util::diag_log::DiagnosticsLog;
use util::stream_guard::StreamGuard;

fn main() {
    let diagnostics_log = DiagnosticsLog::new();
    // Dos salidas para el mismo log: la consola de siempre (con color, para desarrollo)
    // y una copia en memoria acotada (regla R4) que `commands::export_diagnostics_log`
    // puede volcar a fichero cuando el usuario lo pida (F9) — ninguna de las dos ve una
    // URL de proveedor sin redactar antes, así que no hace falta redactar otra vez aquí
    // (ver `util::diag_log`).
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(diagnostics_log.clone()))
        .init();

    let listener =
        StdTcpListener::bind("127.0.0.1:0").expect("no se pudo reservar un puerto local");
    listener
        .set_nonblocking(true)
        .expect("no se pudo configurar el socket local en modo no bloqueante");
    let port = listener
        .local_addr()
        .expect("el socket local no tiene dirección asignada")
        .port();

    tracing::info!(port, "servidor local reservado");

    let stream_guard = StreamGuard::new();

    tauri::Builder::default()
        .manage(ServerPort(port))
        .manage(stream_guard.clone())
        .manage(diagnostics_log)
        .setup(move |app| {
            let app_data_dir = app.path().app_data_dir().expect("no se pudo resolver el directorio de datos");
            let image_cache_dir = app_data_dir.join("images");
            let db_path = db::schema::resolve_db_path(app.handle()).expect("no se pudo resolver la ruta de la base de datos");

            let listener = listener
                .try_clone()
                .expect("no se pudo clonar el socket local");
            let stream_guard = stream_guard.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = serve(listener, image_cache_dir, db_path, stream_guard).await {
                    tracing::error!(error = %err, "el servidor local se detuvo inesperadamente");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_server_port,
            list_providers,
            validate_xtream,
            validate_m3u,
            add_provider,
            sync_provider,
            set_tmdb_api_key,
            has_tmdb_api_key,
            match_tmdb,
            export_diagnostics_log,
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar VIBE IPTV");
}
