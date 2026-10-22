//! Comandos Tauri que conectan la interfaz con el núcleo. Provisional: hasta que exista
//! la API HTTP de F4, la gestión de proveedores y la sincronización se piden por aquí en
//! vez de por `/api/*`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use crate::catalog::epg;
use crate::catalog::tmdb::{self, TmdbClient, TMDB_API_KEY_CREDENTIAL_REF};
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
    add_provider_inner(app, stream_guard.inner().clone(), input).await
}

async fn add_provider_inner(app: AppHandle, stream_guard: Arc<StreamGuard>, input: AddProviderInput) -> CmdResult<ProviderSummary> {
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

    let source = build_source_for_provider(row).await?;
    if !spawn_sync(app, provider_id, source, stream_guard) {
        return Err("Ya se está sincronizando este proveedor.".to_string());
    }
    Ok(())
}

/// Final de una sincronización, con o sin éxito, para Ajustes (`sync://finished`).
/// Antes un fallo solo quedaba en el log: en el iPhone, "Sincronizar" parecía no hacer
/// nada (F15).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SyncFinished {
    #[ts(type = "number")]
    pub provider_id: i64,
    /// `None` si fue bien; si no, el motivo para enseñárselo al usuario.
    pub error: Option<String>,
    #[ts(type = "number")]
    pub channels: i64,
    #[ts(type = "number")]
    pub movies: i64,
    #[ts(type = "number")]
    pub series: i64,
}

/// Prueba de punta a punta en el simulador de iOS (`ios/qa/`): con `VIBE_QA_E2E`, la app
/// arranca con un perfil y el proveedor falso de `VIBE_QA_PROVIDER_URL` (usuario y
/// contraseña "qa"), ya sincronizándose. Solo en compilaciones de depuración: la app que se
/// instala nunca lo mira.
#[cfg(debug_assertions)]
pub fn seed_qa_e2e(app: AppHandle, stream_guard: Arc<StreamGuard>) {
    if std::env::var("VIBE_QA_E2E").is_err() {
        return;
    }
    let base_url = std::env::var("VIBE_QA_PROVIDER_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_string());
    tauri::async_runtime::spawn(async move {
        let Ok(conn) = schema::open_connection(&app) else { return };
        if crate::db::reads::count_profiles(&conn).unwrap_or(0) == 0 {
            let _ = queries::insert_profile(&conn, "QA", None, true);
        }
        if queries::list_providers(&conn).map(|rows| rows.is_empty()).unwrap_or(false) {
            let input = AddProviderInput {
                kind: ProviderKind::Xtream,
                name: "Proveedor de pruebas".into(),
                base_url,
                username: Some("qa".into()),
                password: Some("qa".into()),
                m3u_url: None,
                user_agent: None,
            };
            if let Err(err) = add_provider_inner(app, stream_guard, input).await {
                tracing::error!(error = %err, "no se pudo dar de alta el proveedor de pruebas");
            }
        }
    });
}

/// Pruebas del simulador: `true` si la app arrancó en modo de punta a punta (`VIBE_QA_E2E`).
/// En la app que se instala (compilación de lanzamiento) siempre `false`.
#[tauri::command]
pub fn qa_e2e_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var("VIBE_QA_E2E").is_ok()
}

/// Pruebas del simulador: guarda el mapa de la pantalla web en `qa-ui.json` (datos de la
/// app) para que `ios/qa/e2e-qa.sh` encuentre los botones: idb no ve dentro del WebView.
#[tauri::command]
pub fn qa_ui_snapshot(app: AppHandle, json: String) {
    if !qa_e2e_enabled() {
        return;
    }
    let Ok(dir) = app.path().app_data_dir() else { return };
    let tmp = dir.join("qa-ui.json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join("qa-ui.json"));
    }
}

/// Doce horas desde la última sincronización con éxito (RF2 del PRD): pasado ese
/// tiempo, el catálogo y sobre todo el EPG envejecen en silencio si el usuario no
/// pulsa "Sincronizar" a mano en Ajustes.
const AUTO_SYNC_THRESHOLD_SECONDS: i64 = 12 * 3600;

/// Llamado una vez al arrancar la app (`main.rs`, `.setup()`): sincroniza en segundo
/// plano cualquier proveedor habilitado que no se haya sincronizado nunca, o hace más
/// de 12 horas. No bloquea el arranque de la ventana — se dispara y se olvida, igual
/// que un `sync_provider` manual, con el mismo `StreamGuard` de por medio (R1: se
/// pausa sola si hay reproducción activa).
pub async fn auto_sync_stale_providers(app: AppHandle, stream_guard: Arc<StreamGuard>) {
    let providers = {
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo abrir la base de datos para la sincronización automática");
                return;
            }
        };
        match queries::list_providers(&conn) {
            Ok(rows) => rows,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo listar proveedores para la sincronización automática");
                return;
            }
        }
    };

    let now = crate::db::now_epoch();
    for provider in providers {
        if !provider.enabled {
            continue;
        }
        let is_stale = provider.last_sync_at.is_none_or(|synced_at| now - synced_at > AUTO_SYNC_THRESHOLD_SECONDS);
        if !is_stale {
            continue;
        }
        let provider_id = provider.id;
        match build_source_for_provider(provider).await {
            Ok(source) => {
                tracing::info!(provider_id, "sincronización automática (más de 12h sin sincronizar)");
                spawn_sync(app.clone(), provider_id, source, Arc::clone(&stream_guard));
            }
            Err(err) => tracing::warn!(provider_id, error = %err, "no se pudo reconstruir la fuente para la sincronización automática"),
        }
    }
}

/// Borra un proveedor y todo su catálogo asociado (canales, categorías, y las fuentes
/// de sus películas/series — `queries::delete_provider`, ver su comentario para el
/// porqué de la cascada). No hay confirmación aquí: la pide el frontend antes de
/// llamar, porque no hay vuelta atrás sin resincronizar desde cero.
#[tauri::command]
pub fn delete_provider(app: AppHandle, provider_id: i64) -> CmdResult<()> {
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let provider = queries::get_provider(&conn, provider_id).map_err(|err| err.to_string())?;

    queries::delete_provider(&conn, provider_id).map_err(|err| err.to_string())?;
    // Un proveedor nuevo puede heredar el mismo id: que no herede también su lista.
    if let Ok(mut cache) = M3U_CACHE.lock() {
        cache.retain(|(id, _)| *id != provider_id);
    }
    queries::delete_orphaned_movies_and_series(&conn).map_err(|err| err.to_string())?;
    queries::rebuild_search_index(&conn).map_err(|err| err.to_string())?;

    // Si falla el borrado de la credencial, no deshace el borrado del proveedor: ya no
    // hay nada que la use, y el usuario puede seguir intentándolo desde Ajustes en el
    // peor caso quedaría un secreto huérfano en el keychain, no un dato inconsistente.
    if let Some(credential_ref) = provider.and_then(|p| p.credential_ref) {
        if let Err(err) = keychain::delete_secret(&credential_ref) {
            tracing::warn!(error = %err, "no se pudo borrar la credencial del proveedor eliminado");
        }
    }

    Ok(())
}

/// Reconstruye el `Source` de un proveedor ya guardado, leyendo su credencial del
/// keychain. Reutilizado por `sync_provider` y por la ficha de serie de F4 (bajo
/// demanda, cuando toca refrescar `series_detail`).
pub(crate) async fn build_source_for_provider(row: queries::ProviderRow) -> CmdResult<Arc<dyn Source + Send + Sync>> {
    match row.kind {
        ProviderKind::Xtream => {
            let username = row.username.unwrap_or_default();
            let credential_ref = row.credential_ref.ok_or("proveedor sin credenciales guardadas")?;
            let password = keychain::read_secret(&credential_ref).map_err(|err| err.to_string())?;
            Ok(Arc::new(XtreamSource::new(row.base_url, username, password, None).map_err(|err| err.to_string())?))
        }
        ProviderKind::M3u => {
            if let Some(source) = cached_m3u_source(row.id) {
                return Ok(source);
            }
            let credential_ref = row.credential_ref.ok_or("proveedor sin URL guardada")?;
            let url = keychain::read_secret(&credential_ref).map_err(|err| err.to_string())?;
            let source = Arc::new(fetch_m3u_source(&url).await?);
            if let Ok(mut cache) = M3U_CACHE.lock() {
                cache.retain(|(id, _)| *id != row.id);
                cache.push((row.id, (std::time::Instant::now(), Arc::clone(&source))));
            }
            Ok(source)
        }
    }
}

/// Última lista M3U descargada de cada proveedor. Una lista de panel puede pesar más de
/// 70 MB (medido con la del usuario: 73 MB, 12 s), y se descargaba entera en cada
/// reproducción solo para buscar una URL. Se reutiliza un rato, incluida la que deja la
/// sincronización; pasado ese rato se descarga otra vez, por si el panel caduca las URLs
/// firmadas que pone en ella.
static M3U_CACHE: std::sync::Mutex<Vec<CachedM3u>> = std::sync::Mutex::new(Vec::new());
/// Proveedor, cuándo se descargó y la lista ya parseada.
type CachedM3u = (i64, (std::time::Instant, Arc<M3uSource>));
const M3U_REUSE_FOR: std::time::Duration = std::time::Duration::from_secs(30 * 60);

fn cached_m3u_source(provider_id: i64) -> Option<Arc<dyn Source + Send + Sync>> {
    let mut cache = M3U_CACHE.lock().ok()?;
    cache.retain(|(_, (fetched_at, _))| fetched_at.elapsed() < M3U_REUSE_FOR);
    let source = cache.iter().find(|(id, _)| *id == provider_id).map(|(_, (_, source))| Arc::clone(source))?;
    Some(source)
}

#[tauri::command]
pub fn set_tmdb_api_key(app: AppHandle, api_key: String) -> CmdResult<()> {
    keychain::store_secret(TMDB_API_KEY_CREDENTIAL_REF, &api_key).map_err(|err| err.to_string())?;
    spawn_tmdb_matching(app);
    Ok(())
}

#[tauri::command]
pub fn has_tmdb_api_key() -> bool {
    keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF).is_ok()
}

/// Botón manual "Buscar en TMDB": hace falta cuando la clave se añade después de haber
/// sincronizado ya (si no, los pendientes se quedarían así para siempre).
#[tauri::command]
pub fn match_tmdb(app: AppHandle) -> CmdResult<()> {
    if !has_tmdb_api_key() {
        return Err("no hay clave de API de TMDB guardada todavía".to_string());
    }
    spawn_tmdb_matching(app);
    Ok(())
}

/// Vuelca a fichero las últimas líneas de log capturadas (F9). Ya vienen redactadas: el
/// `DiagnosticsLog` solo copia lo que el `fmt` layer normal de `tracing` produjo, y esa
/// capa nunca ve una URL de proveedor sin redactar (regla R5, ver `util::diag_log`).
#[tauri::command]
pub fn export_diagnostics_log(app: AppHandle, log: State<'_, crate::util::diag_log::DiagnosticsLog>) -> CmdResult<String> {
    let dir = app.path().app_data_dir().map_err(|_| "no se pudo resolver el directorio de datos".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let path = dir.join("diagnostico.log");
    std::fs::write(&path, log.snapshot()).map_err(|err| err.to_string())?;
    Ok(path.display().to_string())
}

/// Una sola pasada de TMDB a la vez: dos a la vez pedirían los mismos títulos dos veces y
/// gastarían el límite de peticiones de TMDB.
static MATCHING_TMDB: AtomicBool = AtomicBool::new(false);

struct MatchingGuard;
impl Drop for MatchingGuard {
    fn drop(&mut self) {
        MATCHING_TMDB.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Al abrir la app: si una pasada de TMDB quedó a medias (la app se cerró), sigue con lo
/// pendiente en vez de esperar a la próxima sincronización, que puede tardar 12 h.
pub fn resume_tmdb_matching(app: AppHandle) {
    spawn_tmdb_matching(app);
}

fn spawn_tmdb_matching(app: AppHandle) {
    if MATCHING_TMDB.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return; // ya hay una pasada en marcha: recogerá lo pendiente.
    }
    tauri::async_runtime::spawn(async move {
        let _matching = MatchingGuard;
        let api_key = match keychain::read_secret(TMDB_API_KEY_CREDENTIAL_REF) {
            Ok(key) => key,
            Err(_) => return, // sin clave todavía: nada que hacer, no es un error
        };
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo abrir la base de datos para emparejar con TMDB");
                return;
            }
        };

        let client = Arc::new(TmdbClient::new(api_key));
        let (movie_genres, tv_genres) = tmdb::fetch_genre_names(&client).await;
        let _ = queries::upsert_genre_names(&conn, "movie", &movie_genres);
        let _ = queries::upsert_genre_names(&conn, "tv", &tv_genres);
        let app_for_progress = app.clone();
        let on_progress: Arc<dyn Fn(u32, u32) + Send + Sync> = Arc::new(move |current, total| {
            let progress = sync::SyncProgress { provider_id: 0, stage: "tmdb".to_string(), current, total };
            let _ = app_for_progress.emit("sync://progress", &progress);
        });

        match tmdb::run_matching_pass(conn, Arc::clone(&client), Arc::new(AtomicBool::new(false)), Arc::clone(&on_progress)).await {
            Ok(summary) => {
                tracing::info!(matched = summary.matched, total = summary.total, "emparejamiento con TMDB completado");
                // Títulos recién emparejados: nuevos géneros, notas y semillas para las
                // recomendaciones de todos los perfiles y para las filas de las pantallas.
                if summary.matched > 0 {
                    crate::catalog::recommendations::invalidate_all().await;
                    crate::catalog::browse::clear_index_cache();
                }
            }
            Err(err) => tracing::error!(error = %err, "emparejamiento con TMDB fallido"),
        }

        // Después, la clasificación por edad de todo lo emparejado (F14): sin ella, un
        // perfil infantil no ve el título.
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo abrir la base de datos para los extras de TMDB");
                return;
            }
        };
        match tmdb::run_extras_pass(conn, client, Arc::new(AtomicBool::new(false)), on_progress).await {
            Ok(stored) => tracing::info!(stored, "clasificación por edad y extras de TMDB al día"),
            Err(err) => tracing::error!(error = %err, "paso de extras de TMDB fallido"),
        }
    });
}

async fn fetch_m3u_source(url: &str) -> CmdResult<M3uSource> {
    let text = fetch_m3u_text(url).await?;
    M3uSource::parse(std::io::Cursor::new(text.into_bytes())).map_err(|err| err.to_string())
}

/// Descarga una lista M3U con la misma identificación de reproductor que el resto de
/// peticiones al proveedor. Un código de error, o algo que no es una lista, es un error
/// con el motivo: antes la página de "prohibido" de un panel se leía como una lista vacía
/// y la validación decía "0 canales, 0 películas, 0 series" como si todo fuera bien.
async fn fetch_m3u_text(url: &str) -> CmdResult<String> {
    let client = reqwest::Client::builder()
        .user_agent(crate::sources::xtream::DEFAULT_USER_AGENT)
        .build()
        .map_err(|err| err.to_string())?;
    let response = client.get(url).send().await.map_err(|_| format!("no se pudo descargar {}", redact(url)))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.bytes().await.unwrap_or_default();
        if let Some(reason) = crate::sources::xtream::rejection_reason(&body) {
            return Err(reason);
        }
        return Err(format!("el proveedor respondió {status} al pedir la lista {}", redact(url)));
    }
    let text = response.text().await.map_err(|_| format!("no se pudo leer {}", redact(url)))?;
    if !text.trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U") {
        return Err(format!("lo que devuelve {} no es una lista M3U", redact(url)));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::Router;

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn an_m3u_download_rejected_by_the_provider_is_an_error_not_an_empty_list() {
        let base = serve(
            Router::new()
                .route("/prohibido", get(|| async { (StatusCode::FORBIDDEN, "Forbidden") }))
                .route(
                    "/pais",
                    get(|| async { (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({"message": "Validation failed, Country is not allowed"}))) }),
                )
                .route("/html", get(|| async { "<html>Acceso denegado</html>" }))
                .route(
                    "/lista",
                    get(|headers: HeaderMap| async move {
                        let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                        format!("#EXTM3U\n#EXTINF:-1 group-title=\"G\",{ua}\nhttp://x/1.ts\n")
                    }),
                ),
        )
        .await;

        let err = fetch_m3u_text(&format!("{base}/prohibido")).await.unwrap_err();
        assert!(err.contains("403"), "{err}");
        let err = fetch_m3u_text(&format!("{base}/pais")).await.unwrap_err();
        assert!(err.contains("país"), "{err}");
        let err = fetch_m3u_text(&format!("{base}/html")).await.unwrap_err();
        assert!(err.contains("no es una lista M3U"), "{err}");
        let text = fetch_m3u_text(&format!("{base}/lista")).await.unwrap();
        assert!(text.contains(crate::sources::xtream::DEFAULT_USER_AGENT), "la descarga va sin identificarse: {text}");
    }
}

/// Proveedores con una sincronización en marcha. Una sincronización se pausa (no se
/// cancela) mientras hay reproducción, así que puede durar lo que dure una película; sin
/// esto, la comprobación horaria o el botón de Ajustes lanzarían otra del mismo proveedor
/// en paralelo, escribiendo las mismas tablas a la vez.
static SYNCING: std::sync::Mutex<Vec<i64>> = std::sync::Mutex::new(Vec::new());

/// Quita el proveedor de `SYNCING` al terminar, también si la tarea entra en pánico.
struct SyncingGuard(i64);
impl Drop for SyncingGuard {
    fn drop(&mut self) {
        if let Ok(mut syncing) = SYNCING.lock() {
            syncing.retain(|id| *id != self.0);
        }
    }
}

/// Lanza la sincronización en segundo plano; `false` si ya había una de ese proveedor.
fn spawn_sync(app: AppHandle, provider_id: i64, source: Arc<dyn Source + Send + Sync>, stream_guard: Arc<StreamGuard>) -> bool {
    {
        let Ok(mut syncing) = SYNCING.lock() else { return false };
        if syncing.contains(&provider_id) {
            tracing::info!(provider_id, "ya hay una sincronización en marcha de este proveedor; no se lanza otra");
            return false;
        }
        syncing.push(provider_id);
    }
    let syncing_guard = SyncingGuard(provider_id);
    let app_for_progress = app.clone();
    let source_for_epg = Arc::clone(&source);
    tauri::async_runtime::spawn(async move {
        let _syncing_guard = syncing_guard;
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(provider_id, error = %err, "no se pudo abrir la base de datos para sincronizar");
                let finished = SyncFinished { provider_id, error: Some(err.to_string()), channels: 0, movies: 0, series: 0 };
                let _ = app.emit("sync://finished", &finished);
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let result = sync::run(conn, provider_id, source, stream_guard, cancel, move |progress| {
            let _ = app_for_progress.emit("sync://progress", &progress);
        })
        .await;

        match result {
            Ok((_, summary)) => {
                tracing::info!(provider_id, channels = summary.channels, movies = summary.movies, series = summary.series, "sincronización completada");
                let finished =
                    SyncFinished { provider_id, error: None, channels: summary.channels, movies: summary.movies, series: summary.series };
                let _ = app.emit("sync://finished", &finished);
                // Pasos 6 y 8 de docs/ARCHITECTURE.md: en segundo plano, sin bloquear el
                // catálogo ya sincronizado. Si no hay clave de TMDB todavía, la primera
                // no hace nada; si la fuente no ofrece XMLTV, tampoco la segunda.
                spawn_tmdb_matching(app.clone());
                spawn_epg_sync(app, provider_id, source_for_epg);
            }
            Err(err) => {
                tracing::error!(provider_id, error = %err, "sincronización fallida");
                let finished = SyncFinished { provider_id, error: Some(err.to_string()), channels: 0, movies: 0, series: 0 };
                let _ = app.emit("sync://finished", &finished);
            }
        }
    });
    true
}

/// Paso 8 de `docs/ARCHITECTURE.md`: descarga e importa la guía XMLTV tras cada
/// sincronización de catálogo. `None` si la fuente no ofrece EPG (M3U sin
/// `x-tvg-url`): no es un error, solo no hay nada que importar.
fn spawn_epg_sync(app: AppHandle, provider_id: i64, source: Arc<dyn Source + Send + Sync>) {
    tauri::async_runtime::spawn(async move {
        let conn = match schema::open_connection(&app) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo abrir la base de datos para la guía EPG");
                return;
            }
        };
        let user_agent = crate::sources::xtream::DEFAULT_USER_AGENT.to_string();
        match epg::run_epg_sync(conn, provider_id, source, user_agent, std::env::temp_dir()).await {
            Ok((_, Some(summary))) => {
                tracing::info!(imported = summary.imported, canales_sin_epg = summary.channels_without_epg, "guía EPG importada");
                // F "premium": antes esto solo se veía en el log — un fallo mudo típico
                // (canales sin guía emparejada) que nadie notaba a menos que mirase la
                // consola. Ahora también llega a Ajustes.
                let _ = app.emit("epg://summary", &summary);
            }
            Ok((_, None)) => tracing::info!("la fuente no ofrece guía XMLTV"),
            Err(err) => tracing::error!(error = %err, "importación de la guía EPG fallida"),
        }
    });
}
