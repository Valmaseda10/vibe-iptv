//! En iPhone/iPad no hay actualizador: la app se instala aparte (ver docs/IOS.md). Mismos
//! comandos que `updater.rs`, vacíos, para que la interfaz sea la misma.

use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;

use crate::util::stream_guard::StreamGuard;

/// Sin estado en móvil.
#[derive(Default)]
pub struct PendingUpdate;

/// Lo que ve la interfaz de una actualización lista.
#[derive(Clone, Serialize)]
pub struct UpdateReady {
    pub version: String,
    pub notes: Option<String>,
}

/// No hace nada en móvil.
pub fn start(_app: AppHandle, _stream_guard: Arc<StreamGuard>) {}

/// Comando Tauri: nunca hay versión lista en móvil.
#[tauri::command]
pub fn update_ready() -> Option<UpdateReady> {
    None
}

/// Comando Tauri: no aplica en móvil.
#[tauri::command]
pub fn install_update() -> Result<(), String> {
    Err("Las actualizaciones de iPhone e iPad se instalan aparte.".into())
}

/// Resultado de mirar si hay versión nueva: en móvil nunca se mira.
#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateCheck {
    Failed { message: String },
}

/// Comando Tauri: en iPhone e iPad la versión nueva se instala aparte.
#[tauri::command]
pub async fn check_for_update() -> Result<UpdateCheck, String> {
    Ok(UpdateCheck::Failed { message: "En iPhone y iPad la versión nueva se instala aparte.".into() })
}
