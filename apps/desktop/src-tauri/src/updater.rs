//! Actualizaciones automáticas de la app de escritorio (2026-10-30, ver docs/DECISIONS.md).
//!
//! Se publican en GitHub Releases, firmadas con una clave que no está en el repositorio
//! (`tools/publicar-version.mjs`). La app mira si hay versión nueva un rato después de
//! abrirse y cada pocas horas, la descarga en segundo plano solo cuando no hay nada
//! reproduciéndose (R1: no competir con el stream) y avisa a la interfaz; se instala
//! cuando el usuario pulsa «Reiniciar», nunca a mitad de una película.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::util::stream_guard::StreamGuard;

/// Evento con la versión lista para instalar.
const EVENT_READY: &str = "updater://ready";
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(45);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
const RETRY_WHILE_PLAYING: Duration = Duration::from_secs(5 * 60);

/// Versión descargada y lista para instalar.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<(Update, Vec<u8>)>>);

/// Lo que ve la interfaz de una actualización lista.
#[derive(Clone, Serialize)]
pub struct UpdateReady {
    pub version: String,
    pub notes: Option<String>,
}

/// Arranca la comprobación periódica. En desarrollo no hace nada.
pub fn start(app: AppHandle, stream_guard: Arc<StreamGuard>) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            if stream_guard.is_busy() {
                tokio::time::sleep(RETRY_WHILE_PLAYING).await;
                continue;
            }
            check_and_download(&app, &stream_guard).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

async fn check_and_download(app: &AppHandle, stream_guard: &StreamGuard) {
    let pending = app.state::<PendingUpdate>();
    if pending.0.lock().map(|slot| slot.is_some()).unwrap_or(true) {
        return;
    }
    let update = match app.updater().map_err(|err| err.to_string()) {
        Ok(updater) => match updater.check().await {
            Ok(Some(update)) => update,
            Ok(None) => return,
            // Sin red o GitHub caído: se vuelve a mirar en la siguiente vuelta.
            Err(err) => {
                tracing::info!(error = %err, "no se pudo comprobar si hay versión nueva");
                return;
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "actualizador sin configurar");
            return;
        }
    };
    if stream_guard.is_busy() {
        return;
    }
    let version = update.version.clone();
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, version, "no se pudo descargar la versión nueva");
            return;
        }
    };
    tracing::info!(version, "versión nueva descargada, lista para instalar");
    let ready = UpdateReady { version, notes: update.body.clone() };
    if let Ok(mut slot) = pending.0.lock() {
        *slot = Some((update, bytes));
    }
    let _ = app.emit(EVENT_READY, ready);
}

/// Comando Tauri: la versión lista para instalar, si hay (para la interfaz que se monta
/// después del evento).
#[tauri::command]
pub fn update_ready(pending: tauri::State<'_, PendingUpdate>) -> Option<UpdateReady> {
    let slot = pending.0.lock().ok()?;
    slot.as_ref().map(|(update, _)| UpdateReady { version: update.version.clone(), notes: update.body.clone() })
}

/// Comando Tauri: instala la versión descargada y reinicia. En Windows el instalador
/// cierra la app (con ella, ffmpeg y mpv: van atados a su vida, R1) y la vuelve a abrir.
#[tauri::command]
pub fn install_update(app: AppHandle, pending: tauri::State<'_, PendingUpdate>) -> Result<(), String> {
    let taken = pending.0.lock().map_err(|_| "estado de actualización no disponible".to_string())?.take();
    let Some((update, bytes)) = taken else {
        return Err("No hay ninguna versión nueva descargada.".into());
    };
    crate::util::child_processes::let_new_processes_outlive_app();
    update.install(bytes).map_err(|err| format!("No se pudo instalar la versión nueva: {err}"))?;
    app.restart();
}
