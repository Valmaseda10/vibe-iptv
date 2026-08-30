//! Comandos Tauri que conectan la interfaz con el núcleo. Provisional: hasta que exista
//! la API HTTP de F4, la gestión de proveedores y la sincronización se piden por aquí en
//! vez de por `/api/*`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::catalog::sync;
use crate::db::queries::{self, ProviderKind};
use crate::db::schema;
use crate::sources::m3u::{self, M3uSource};
use crate::sources::xtream::XtreamSource;
use crate::sources::Source;
use crate::util::redact;
use crate::util::keychain;
use crate::util::stream_guard::StreamGuard;

type CmdResult<T> = std::result::Result<T, String>;

/// Vista de un proveedor para la interfaz: nunca lleva `credential_ref` (regla R5).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct ProviderSummary {
    // i64 -> "number", no "bigint": Tauri manda esto por JSON normal (vía JSON.parse),
    // así que en tiempo de ejecución nunca es un bigint de verdad pese a lo que ts-rs
    // pondría por defecto. Todos estos valores caben sobrados en Number.MAX_SAFE_INTEGER.
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url_display: String,
    pub username: Option<String>,
    #[ts(type = "number")]
    pub max_connections: i64,
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    pub enabled: bool,
    #[ts(type = "number | null")]
    pub last_sync_at: Option<i64>,
}

impl From<queries::ProviderRow> for ProviderSummary {
    fn from(row: queries::ProviderRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            kind: row.kind,
            base_url_display: row.base_url,
            username: row.username,
            max_connections: row.max_connections,
            expires_at: row.expires_at,
            enabled: row.enabled,
            last_sync_at: row.last_sync_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct AccountStatus {
    pub auth_ok: bool,
    pub status: String,
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct XtreamSuggestion {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct M3uValidation {
    pub channel_count: u32,
    pub movie_count: u32,
    pub series_count: u32,
    /// Presente si la URL es en realidad un `get.php` de Xtream (ver
    /// `docs/XTREAM-API.md`). La interfaz ofrece usar estos datos para darlo de alta
    /// como proveedor Xtream en vez de M3U.
    pub suggested_xtream: Option<XtreamSuggestion>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct AddProviderInput {
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub m3u_url: Option<String>,
    pub user_agent: Option<String>,
}

#[tauri::command]
pub fn list_providers(app: AppHandle) -> CmdResult<Vec<ProviderSummary>> {
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let rows = queries::list_providers(&conn).map_err(|err| err.to_string())?;
    Ok(rows.into_iter().map(ProviderSummary::from).collect())
}

#[tauri::command]
pub async fn validate_xtream(
    base_url: String,
    username: String,
    password: String,
    user_agent: Option<String>,
) -> CmdResult<AccountStatus> {
    let source = XtreamSource::new(base_url, username, password, user_agent.as_deref()).map_err(|err| err.to_string())?;
    let info = source.account_info().await.map_err(|err| err.to_string())?;
    Ok(AccountStatus {
        auth_ok: info.auth_ok,
        status: info.status,
        expires_at: info.expires_at,
        max_connections: info.max_connections,
    })
}

#[tauri::command]
pub async fn validate_m3u(url: String) -> CmdResult<M3uValidation> {
    let suggested_xtream = m3u::detect_embedded_xtream_credentials(&url).map(|creds| XtreamSuggestion {
        base_url: creds.base_url,
        username: creds.username,
        password: creds.password,
    });

    let source = fetch_m3u_source(&url).await?;
    let channel_count = source.live_channels().await.map_err(|err| err.to_string())?.len() as u32;
    let movie_count = source.movies().await.map_err(|err| err.to_string())?.len() as u32;
    let series_count = source.series().await.map_err(|err| err.to_string())?.len() as u32;

    Ok(M3uValidation { channel_count, movie_count, series_count, suggested_xtream })
}

#[tauri::command]
pub async fn add_provider(
    app: AppHandle,
    stream_guard: State<'_, Arc<StreamGuard>>,
    input: AddProviderInput,
) -> CmdResult<ProviderSummary> {
    let stream_guard: Arc<StreamGuard> = stream_guard.inner().clone();

    let (source, base_url_for_db, username_for_db, secret): (Arc<dyn Source + Send + Sync>, String, Option<String>, Option<String>) =
        match input.kind {
            ProviderKind::Xtream => {
                let username = input.username.clone().ok_or("falta el usuario")?;
                let password = input.password.clone().ok_or("falta la contraseña")?;
                let source = XtreamSource::new(input.base_url.clone(), username.clone(), password.clone(), input.user_agent.as_deref())
                    .map_err(|err| err.to_string())?;
                (Arc::new(source), input.base_url.clone(), Some(username), Some(password))
            }
            ProviderKind::M3u => {
                let url = input.m3u_url.clone().ok_or("falta la URL de la lista M3U")?;
                let source = fetch_m3u_source(&url).await?;
                (Arc::new(source), redact(&url), None, Some(url))
            }
        };

    let account = source.account_info().await.map_err(|err| err.to_string())?;
    if !account.auth_ok {
        return Err("el proveedor rechazó las credenciales".to_string());
    }

    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let provider_id = queries::insert_provider(
        &conn,
        input.kind,
        &input.name,
        &base_url_for_db,
        username_for_db.as_deref(),
        account.max_connections as i64,
        account.expires_at,
    )
    .map_err(|err| err.to_string())?;

    if let Some(secret) = secret {
        let credential_ref = format!("provider-{provider_id}");
        keychain::store_secret(&credential_ref, &secret).map_err(|err| err.to_string())?;
        queries::set_provider_credential_ref(&conn, provider_id, &credential_ref).map_err(|err| err.to_string())?;
    }

    let provider_row = queries::get_provider(&conn, provider_id)
        .map_err(|err| err.to_string())?
        .ok_or("proveedor recién creado no encontrado")?;

    spawn_sync(app, provider_id, Arc::clone(&source), stream_guard);

    Ok(ProviderSummary::from(provider_row))
}

#[tauri::command]
pub async fn sync_provider(app: AppHandle, stream_guard: State<'_, Arc<StreamGuard>>, provider_id: i64) -> CmdResult<()> {
    let stream_guard: Arc<StreamGuard> = stream_guard.inner().clone();
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let row = queries::get_provider(&conn, provider_id)
        .map_err(|err| err.to_string())?
        .ok_or("proveedor no encontrado")?;

    let source: Arc<dyn Source + Send + Sync> = match row.kind {
        ProviderKind::Xtream => {
            let username = row.username.unwrap_or_default();
            let credential_ref = row.credential_ref.ok_or("proveedor sin credenciales guardadas")?;
            let password = keychain::read_secret(&credential_ref).map_err(|err| err.to_string())?;
            Arc::new(XtreamSource::new(row.base_url, username, password, None).map_err(|err| err.to_string())?)
        }
        ProviderKind::M3u => {
            let credential_ref = row.credential_ref.ok_or("proveedor sin URL guardada")?;
            let url = keychain::read_secret(&credential_ref).map_err(|err| err.to_string())?;
            Arc::new(fetch_m3u_source(&url).await?)
        }
    };

    spawn_sync(app, provider_id, source, stream_guard);
    Ok(())
}

async fn fetch_m3u_source(url: &str) -> CmdResult<M3uSource> {
    let response = reqwest::get(url).await.map_err(|_| format!("no se pudo descargar {}", redact(url)))?;
    let text = response.text().await.map_err(|_| format!("no se pudo leer {}", redact(url)))?;
    M3uSource::parse(std::io::Cursor::new(text.into_bytes())).map_err(|err| err.to_string())
}

fn spawn_sync(app: AppHandle, provider_id: i64, source: Arc<dyn Source + Send + Sync>, stream_guard: Arc<StreamGuard>) {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn(async move {
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(provider_id, error = %err, "no se pudo abrir la base de datos para sincronizar");
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let result = sync::run(conn, provider_id, source, stream_guard, cancel, move |progress| {
            let _ = app_for_progress.emit("sync://progress", &progress);
        })
        .await;

        match result {
            Ok((_, summary)) => tracing::info!(provider_id, channels = summary.channels, movies = summary.movies, series = summary.series, "sincronización completada"),
            Err(err) => tracing::error!(provider_id, error = %err, "sincronización fallida"),
        }
    });
}
