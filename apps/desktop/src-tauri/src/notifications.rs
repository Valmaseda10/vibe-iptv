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

/// Si este perfil quiere avisos de episodios nuevos en este aparato (F17, parte 4).
#[tauri::command]
pub fn new_episode_alerts(app: AppHandle, profile_id: i64) -> Result<bool, String> {
    let conn = crate::db::schema::open_connection(&app).map_err(|err| err.to_string())?;
    crate::catalog::new_episodes::enabled(&conn, profile_id).map_err(|err| err.to_string())
}

/// Activa o desactiva los avisos de episodios nuevos de un perfil en este aparato.
#[tauri::command]
pub fn set_new_episode_alerts(app: AppHandle, profile_id: i64, enabled: bool) -> Result<(), String> {
    let conn = crate::db::schema::open_connection(&app).map_err(|err| err.to_string())?;
    crate::catalog::new_episodes::set_enabled(&conn, profile_id, enabled).map_err(|err| err.to_string())
}

/// Avisa de los episodios nuevos encontrados.
fn announce(app: &AppHandle, found: &[crate::catalog::new_episodes::NewEpisode]) {
    let mut seen = std::collections::HashSet::new();
    for episode in found {
        // Dos perfiles del aparato siguen la misma serie: un aviso.
        if !seen.insert((episode.series_id, episode.season, episode.number)) {
            continue;
        }
        let body = format!("{}: temporada {}, episodio {}", episode.series_title, episode.season, episode.number);
        if let Err(err) = app.notification().builder().title("Nuevo episodio").body(body).show() {
            tracing::warn!(error = %err, "no se pudo avisar del episodio nuevo");
        }
    }
}

/// Lo más a menudo que se mira si toca comprobar (la comprobación en sí, cada 6 horas).
const NEW_EPISODES_TICK: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Comprueba los episodios nuevos de vez en cuando con la app abierta y, en el iPhone,
/// también cuando iOS la despierta en segundo plano.
pub fn start_new_episode_checks(app: AppHandle, db_path: std::path::PathBuf, guard: std::sync::Arc<crate::util::stream_guard::StreamGuard>) {
    #[cfg(target_os = "ios")]
    background::install(app.clone(), db_path.clone(), guard.clone());
    tauri::async_runtime::spawn(async move {
        // Al abrir, que la app termine de arrancar (y quizá de sincronizar) antes.
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            match crate::catalog::new_episodes::check(db_path.clone(), guard.clone(), false).await {
                Ok(found) => announce(&app, &found),
                Err(err) => tracing::warn!(error = %err, "no se pudieron comprobar los episodios nuevos"),
            }
            tokio::time::sleep(NEW_EPISODES_TICK).await;
        }
    });
}

#[cfg(target_os = "ios")]
mod background {
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use tauri::AppHandle;

    use crate::util::stream_guard::StreamGuard;

    static STATE: OnceLock<(AppHandle, PathBuf, Arc<StreamGuard>)> = OnceLock::new();

    extern "C" {
        fn vibe_background_register();
    }

    pub fn install(app: AppHandle, db_path: PathBuf, guard: Arc<StreamGuard>) {
        let _ = STATE.set((app, db_path, guard));
        // SAFETY: función de Swift sin argumentos; registra la tarea de segundo plano.
        unsafe { vibe_background_register() };
    }

    /// Llamada por iOS (desde Swift) al despertar la app: comprueba (sin esperar las 6 h)
    /// en menos de 25 s, que es lo que da iOS.
    #[no_mangle]
    pub extern "C" fn vibe_background_refresh() -> bool {
        let Some((app, db_path, guard)) = STATE.get().cloned() else { return false };
        let result = tauri::async_runtime::block_on(async move {
            tokio::time::timeout(Duration::from_secs(25), crate::catalog::new_episodes::check(db_path, guard, true)).await
        });
        match result {
            Ok(Ok(found)) => {
                super::announce(&app, &found);
                true
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "episodios nuevos en segundo plano: fallo");
                false
            }
            Err(_) => {
                tracing::info!("episodios nuevos en segundo plano: sin tiempo");
                false
            }
        }
    }
}

/// Solo en desarrollo (pruebas): comprueba ya los episodios nuevos, sin esperar las 6 h,
/// avisa y dice cuántos encontró.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn qa_check_new_episodes(app: AppHandle) -> Result<usize, String> {
    use tauri::Manager;
    let db_path = crate::db::schema::resolve_db_path(&app).map_err(|err| err.to_string())?;
    let guard = app.state::<std::sync::Arc<crate::util::stream_guard::StreamGuard>>().inner().clone();
    let found = crate::catalog::new_episodes::check(db_path, guard, true).await.map_err(|err| err.to_string())?;
    announce(&app, &found);
    Ok(found.len())
}
