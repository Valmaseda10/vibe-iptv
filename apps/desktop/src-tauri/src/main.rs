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
    add_provider, has_tmdb_api_key, list_providers, match_tmdb, set_tmdb_api_key, sync_provider, validate_m3u,
    validate_xtream,
};
use server::{get_server_port, serve, ServerPort};
use tauri::Manager;
use util::stream_guard::StreamGuard;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
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

    tauri::Builder::default()
        .manage(ServerPort(port))
        .manage(StreamGuard::new())
        .setup(move |app| {
            let app_data_dir = app.path().app_data_dir().expect("no se pudo resolver el directorio de datos");
            let image_cache_dir = app_data_dir.join("images");
            let db_path = db::schema::resolve_db_path(app.handle()).expect("no se pudo resolver la ruta de la base de datos");

            let listener = listener
                .try_clone()
                .expect("no se pudo clonar el socket local");
            tauri::async_runtime::spawn(async move {
                if let Err(err) = serve(listener, image_cache_dir, db_path).await {
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
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar VIBE IPTV");
}
