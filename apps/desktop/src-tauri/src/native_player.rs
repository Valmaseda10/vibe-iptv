//! Reproductor nativo de iPhone y iPad (F15). En iOS no hay ffmpeg ni mpv, y el WebView
//! no abre TS ni MKV: la reproducción la hace VLC (MobileVLCKit) en una pantalla nativa
//! escrita en Swift (`src-tauri/ios/VibePlayer.swift`), que se presenta por encima de la
//! interfaz web.
//!
//! El puente son dos funciones C con JSON dentro, sin plugin ni paquete Swift aparte:
//! - `vibe_player_present`, en Swift: abre (o cambia) lo que se reproduce.
//! - [`vibe_player_event`], aquí: Swift avisa del progreso, del cierre, de un gesto de
//!   zapping o de un error, y se reenvía a la interfaz como evento `native-player`.
//!
//! VLC solo recibe la URL del proxy local (`/stream/...?passthrough=true`), nunca la del
//! proveedor: las credenciales no salen del núcleo (R5) y el `StreamGuard` sigue
//! decidiendo la única conexión (R1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// Solo el puente de iOS los usa; fuera de iOS el comando responde `Unsupported`.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
/// Nombre del evento de Tauri con el que llega cada [`NativePlayerEvent`] a la interfaz.
pub const EVENT_NAME: &str = "native-player";

/// Lo que la interfaz pide reproducir.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct NativePlayRequest {
    /// URL del proxy local, con `passthrough=true`.
    pub url: String,
    pub title: String,
    /// Segunda línea: programa en emisión o nombre del episodio.
    pub subtitle: Option<String>,
    /// Segundo por el que empezar (continuar viendo).
    pub start_seconds: f64,
    /// Directo: sin barra de tiempo, y deslizar arriba y abajo cambia de canal.
    pub live: bool,
}

/// Lo que cuenta el reproductor nativo.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export, export_to = "api-types.ts")]
pub enum NativePlayerEvent {
    /// Cada pocos segundos mientras reproduce.
    Progress { position_s: f64, duration_s: Option<f64> },
    /// El usuario cerró el reproductor o el título terminó (`ended`).
    Closed { position_s: f64, duration_s: Option<f64>, ended: bool },
    /// Gesto de zapping en directo: `1` siguiente canal, `-1` anterior.
    Zap { direction: i32 },
    /// VLC no pudo abrir o seguir reproduciendo el stream.
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum NativePlayerError {
    #[error("El reproductor nativo solo existe en iPhone y iPad.")]
    Unsupported,
    #[error("Petición de reproducción no válida.")]
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    InvalidRequest,
}

impl Serialize for NativePlayerError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Parsea un evento llegado de Swift; `None` si no es un evento conocido.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn parse_event(json: &str) -> Option<NativePlayerEvent> {
    serde_json::from_str(json).ok()
}

/// Comando Tauri: abre el reproductor nativo con `request`, o cambia lo que reproduce si
/// ya está abierto (zapping).
#[tauri::command]
pub fn native_player_play(request: NativePlayRequest) -> Result<(), NativePlayerError> {
    platform::present(&request)
}

#[cfg(target_os = "ios")]
mod platform {
    use std::ffi::{c_char, CStr, CString};
    use std::sync::OnceLock;

    use tauri::{AppHandle, Emitter};

    use super::{parse_event, NativePlayRequest, NativePlayerError, EVENT_NAME};

    static APP: OnceLock<AppHandle> = OnceLock::new();

    extern "C" {
        fn vibe_player_present(json: *const c_char);
    }

    /// Guarda el `AppHandle` con el que se reenvían los eventos a la interfaz.
    pub fn install(app: AppHandle) {
        let _ = APP.set(app);
    }

    pub fn present(request: &NativePlayRequest) -> Result<(), NativePlayerError> {
        let json = serde_json::to_string(request).map_err(|_| NativePlayerError::InvalidRequest)?;
        let json = CString::new(json).map_err(|_| NativePlayerError::InvalidRequest)?;
        // SAFETY: Swift copia la cadena antes de volver (la pasa al hilo principal ya
        // convertida en `String`), así que basta con que viva durante la llamada.
        unsafe { vibe_player_present(json.as_ptr()) };
        Ok(())
    }

    /// Llamada desde Swift con un evento en JSON.
    ///
    /// # Safety
    /// `json` tiene que ser una cadena C válida terminada en nulo, viva durante la llamada.
    #[no_mangle]
    pub unsafe extern "C" fn vibe_player_event(json: *const c_char) {
        if json.is_null() {
            return;
        }
        let Ok(text) = CStr::from_ptr(json).to_str() else { return };
        let Some(event) = parse_event(text) else {
            tracing::warn!("evento desconocido del reproductor nativo");
            return;
        };
        if let Some(app) = APP.get() {
            if let Err(err) = app.emit(EVENT_NAME, event) {
                tracing::warn!(error = %err, "no se pudo reenviar un evento del reproductor nativo");
            }
        }
    }
}

#[cfg(not(target_os = "ios"))]
mod platform {
    use super::{NativePlayRequest, NativePlayerError};

    pub fn install(_app: tauri::AppHandle) {}

    pub fn present(_request: &NativePlayRequest) -> Result<(), NativePlayerError> {
        Err(NativePlayerError::Unsupported)
    }
}

pub use platform::install;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_event_swift_sends() {
        assert_eq!(
            parse_event(r#"{"type":"progress","position_s":12.5,"duration_s":5400}"#),
            Some(NativePlayerEvent::Progress { position_s: 12.5, duration_s: Some(5400.0) })
        );
        assert_eq!(
            parse_event(r#"{"type":"closed","position_s":30,"duration_s":null,"ended":false}"#),
            Some(NativePlayerEvent::Closed { position_s: 30.0, duration_s: None, ended: false })
        );
        assert_eq!(parse_event(r#"{"type":"zap","direction":-1}"#), Some(NativePlayerEvent::Zap { direction: -1 }));
        assert_eq!(
            parse_event(r#"{"type":"error","message":"no se pudo abrir"}"#),
            Some(NativePlayerEvent::Error { message: "no se pudo abrir".into() })
        );
        assert_eq!(parse_event(r#"{"type":"otra_cosa"}"#), None);
    }

    #[test]
    fn outside_ios_the_command_says_it_is_not_available() {
        let request = NativePlayRequest { url: "http://127.0.0.1:1/stream/movie/1".into(), title: "T".into(), subtitle: None, start_seconds: 0.0, live: false };
        assert!(matches!(native_player_play(request), Err(NativePlayerError::Unsupported)));
    }
}
