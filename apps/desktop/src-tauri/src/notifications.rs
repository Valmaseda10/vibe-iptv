//! Avisos del sistema (F17): recordatorios de programas, partidos de «Mis equipos» y, más
//! adelante, episodios nuevos. Con el plugin oficial de Tauri.
//!
//! En el iPhone se dejan programados: avisan aunque la app esté cerrada. La interfaz manda
//! la lista entera de lo que tiene que avisar (`notifications_replace_scheduled`) y aquí se
//! cancela lo programado antes y se programa lo nuevo. En el PC, Windows no programa
//! avisos: la interfaz llama a `notifications_show` cuando toca, con la app abierta.

use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};

/// Un aviso programado: id estable (para no repetirlo), texto y cuándo (epoch).
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ScheduledNotice {
    pub id: i32,
    pub title: String,
    pub body: String,
    #[ts(type = "number")]
    pub at: i64,
}

fn permission_name(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        _ => "prompt",
    }
}

/// `granted`, `denied` o `prompt` (aún no se ha preguntado).
#[tauri::command]
pub fn notifications_permission(app: AppHandle) -> String {
    app.notification().permission_state().map(permission_name).unwrap_or("denied").to_string()
}

/// Pide permiso (en el iPhone sale la pregunta del sistema la primera vez).
#[tauri::command]
pub fn notifications_request(app: AppHandle) -> String {
    app.notification().request_permission().map(permission_name).unwrap_or("denied").to_string()
}

/// Un aviso ya.
#[tauri::command]
pub fn notifications_show(app: AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification().builder().title(title).body(body).show().map_err(|err| err.to_string())
}

/// Si este aparato deja avisos programados (iPhone) o los enseña la interfaz al llegar la
/// hora (PC).
#[tauri::command]
pub fn notifications_can_schedule() -> bool {
    cfg!(mobile)
}

/// Cambia los avisos programados por estos (los pasados se ignoran). En el PC no hace nada.
#[tauri::command]
pub fn notifications_replace_scheduled(app: AppHandle, notices: Vec<ScheduledNotice>) -> Result<(), String> {
    #[cfg(mobile)]
    {
        use tauri_plugin_notification::Schedule;
        let notifications = app.notification();
        notifications.cancel_all().map_err(|err| err.to_string())?;
        let now = crate::db::now_epoch();
        for notice in notices.into_iter().filter(|n| n.at > now) {
            let Ok(date) = time::OffsetDateTime::from_unix_timestamp(notice.at) else { continue };
            notifications
                .builder()
                .id(notice.id)
                .title(notice.title)
                .body(notice.body)
                .schedule(Schedule::At { date, repeating: false, allow_while_idle: true })
                .show()
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(mobile))]
    {
        let _ = (app, notices);
        Ok(())
    }
}
