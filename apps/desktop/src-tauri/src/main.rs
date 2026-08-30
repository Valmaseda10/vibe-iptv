// Previene la consola extra en Windows en builds de release; irrelevante en dev.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod db;
mod media;
mod server;
mod sources;
mod util;

use std::net::TcpListener as StdTcpListener;

use server::{get_server_port, serve, ServerPort};

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
        .setup(move |_app| {
            let listener = listener
                .try_clone()
                .expect("no se pudo clonar el socket local");
            tauri::async_runtime::spawn(async move {
                if let Err(err) = serve(listener).await {
                    tracing::error!(error = %err, "el servidor local se detuvo inesperadamente");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_server_port])
        .run(tauri::generate_context!())
        .expect("error al arrancar VIBE IPTV");
}
