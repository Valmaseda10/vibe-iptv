pub mod account;
pub mod api;
pub mod browse;
pub mod live_buffer;
pub mod profiles;
pub mod provider_status;
pub mod routes;
pub mod stream;
pub mod timeshift;
pub mod tv;

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::db::schema;
use crate::sync::backend::CloudBackend;
use crate::sync::engine;
use crate::sync::firebase::{FirebaseBackend, FirebaseConfig};
use crate::util::stream_guard::StreamGuard;

/// El servidor local: su puerto y lo necesario para volver a levantarlo.
///
/// iOS puede dejar inservible el socket de escucha de una app suspendida (al pasar a
/// segundo plano). La interfaz seguía ahí al volver, pero ninguna petición llegaba:
/// "los botones no van" (F15). [`ensure_server`] comprueba que responde y, si no, lo
/// vuelve a abrir con el mismo router.
pub struct ServerControl {
    port: AtomicU16,
    router: tokio::sync::OnceCell<Router>,
    task: tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl ServerControl {
    pub fn new(port: u16) -> Self {
        Self { port: AtomicU16::new(port), router: tokio::sync::OnceCell::new(), task: tokio::sync::Mutex::new(None) }
    }

    /// Primer arranque: guarda el router y sirve en el socket ya reservado.
    pub async fn start(&self, listener: StdTcpListener, router: Router) -> std::io::Result<()> {
        let _ = self.router.set(router.clone());
        let handle = spawn_serving(listener, router)?;
        *self.task.lock().await = Some(handle);
        Ok(())
    }
}

/// Comando Tauri que expone al frontend el puerto efímero en el que escucha el
/// servidor local (regla R2 de CLAUDE.md: nunca un puerto fijo).
#[tauri::command]
pub fn get_server_port(control: tauri::State<'_, Arc<ServerControl>>) -> u16 {
    control.port.load(Ordering::SeqCst)
}

/// Comprueba que el servidor local responde y, si no, lo vuelve a abrir (en el mismo
/// puerto si está libre). Devuelve el puerto en el que escucha.
#[tauri::command]
pub async fn ensure_server(control: tauri::State<'_, Arc<ServerControl>>) -> Result<u16, String> {
    control.ensure().await
}

impl ServerControl {
    /// Ver [`ensure_server`].
    pub async fn ensure(&self) -> Result<u16, String> {
        let mut task = self.task.lock().await;
        let port = self.port.load(Ordering::SeqCst);
        let Some(router) = self.router.get().cloned() else {
            return Ok(port); // todavía arrancando: las peticiones esperan en la cola del socket
        };
        if responds(port).await {
            return Ok(port);
        }
        tracing::warn!(port, "el servidor local no responde; se vuelve a abrir");
        if let Some(old) = task.take() {
            old.abort();
            // El socket viejo se suelta al terminar la tarea abortada.
            let _ = old.await;
        }
        let listener = StdTcpListener::bind(("127.0.0.1", port))
            .or_else(|_| StdTcpListener::bind("127.0.0.1:0"))
            .map_err(|err| err.to_string())?;
        let new_port = listener.local_addr().map_err(|err| err.to_string())?.port();
        *task = Some(spawn_serving(listener, router).map_err(|err| err.to_string())?);
        self.port.store(new_port, Ordering::SeqCst);
        Ok(new_port)
    }
}

async fn responds(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_millis(1500)).build() else {
        return false;
    };
    matches!(client.get(format!("http://127.0.0.1:{port}/api/health")).send().await, Ok(r) if r.status().is_success())
}

fn spawn_serving(listener: StdTcpListener, router: Router) -> std::io::Result<tauri::async_runtime::JoinHandle<()>> {
    // En Windows, un socket duplicado con `try_clone()` no hereda el modo no bloqueante
    // del original (es un atributo de Winsock por handle, no por socket subyacente).
    // Sin esto, `accept()` bloquea el hilo del runtime y el servidor deja de responder
    // sin ningún error visible.
    listener.set_nonblocking(true)?;
    Ok(tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                tracing::error!(error = %err, "no se pudo preparar el socket del servidor local");
                return;
            }
        };
        if let Err(err) = axum::serve(listener, router.into_make_service()).await {
            tracing::error!(error = %err, "el servidor local se detuvo inesperadamente");
        }
    }))
}

/// Prepara la API local (migraciones, estado, motor de la nube) y devuelve su router.
pub async fn build(image_cache_dir: PathBuf, db_path: PathBuf, stream_guard: Arc<StreamGuard>) -> std::io::Result<Router> {

    // Aplica las migraciones aquí, antes de servir nada. Sin esto había una carrera de
    // verdad, no solo teórica: en una instalación nueva (fichero de base de datos
    // inexistente) la primera petición HTTP real llega por `open_readonly_at` (que
    // nunca comprueba migraciones) y podría adelantarse al primer comando Tauri que sí
    // las dispara (`list_providers`, vía `open_connection`).
    {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || -> crate::db::Result<()> {
            schema::open_connection_at(&db_path)?;
            Ok(())
        })
        .await
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other)?;
    }

    crate::media::subtitles::clear_temp_dir();
    timeshift::clear_temp_root();
    live_buffer::clear_temp_root();
    let stream_state = stream::StreamState {
        db_path: db_path.clone(),
        stream_guard,
        // `resolve_binary` mira primero junto al ejecutable (sidecar empaquetado, F10) y
        // cae al PATH si no lo encuentra (desarrollo, o el usuario lo instaló él mismo).
        ffmpeg_path: crate::media::resolve_binary("ffmpeg"),
        ffprobe_path: crate::media::resolve_binary("ffprobe"),
        mpv_path: crate::media::resolve_binary("mpv"),
        last_errors: Default::default(),
        subtitles: Default::default(),
        timeshift: Default::default(),
        live_buffer: Default::default(),
    };
    if stream_state.ffmpeg_path.is_none() || stream_state.ffprobe_path.is_none() {
        tracing::warn!("ffmpeg y/o ffprobe no están en el PATH: el nivel 2 de reproducción no funcionará hasta F10");
    }
    if stream_state.mpv_path.is_none() {
        tracing::warn!("mpv no está en el PATH: el nivel 3 de reproducción no funcionará hasta que se instale");
    }

    // Sincronización con la nube (F11): un único cliente compartido por el motor en
    // segundo plano y la API de cuenta. Arranca después de las migraciones de arriba,
    // que crean la cola de salida.
    let cloud: Arc<dyn CloudBackend> = Arc::new(FirebaseBackend::new(FirebaseConfig::production()));
    engine::start(db_path.clone(), cloud.clone());

    Ok(full_router(image_cache_dir, db_path, stream_state, cloud))
}

/// El router completo: catálogo + salud/imágenes + streaming, con CORS aplicado una
/// sola vez sobre el conjunto. Separado de `serve()` (que además reserva el socket real)
/// para poder probarlo con `oneshot` sin abrir ningún puerto.
///
/// CORS abierto a cualquier origen: el WebView de Tauri sirve el frontend desde un
/// origen distinto al del servidor (`http://localhost:1420` en dev, `tauri://localhost`
/// empaquetado), y este servidor solo escucha en 127.0.0.1 sin cookies ni sesión (R2).
///
/// Tiene que envolver el router ya fusionado, no una de sus mitades por separado: un
/// `.layer()` aplicado antes de `.merge()` no cubre las rutas que se fusionan después.
/// Ese error real llegó a producción (F6): `fetch()` desde el WebView a
/// `/api/playback/tier/*` fallaba en silencio por CORS —`<video src>` no lo notó porque
/// cargar un recurso de vídeo no pasa por CORS, solo `fetch`/`XHR` sí— y el panel de
/// estadísticas del reproductor se quedó mostrando "nivel desconocido" para siempre.
fn full_router(image_cache_dir: PathBuf, db_path: PathBuf, stream_state: stream::StreamState, cloud: Arc<dyn CloudBackend>) -> Router {
    routes::build_router(image_cache_dir, db_path.clone())
        .merge(stream::stream_router(stream_state))
        .merge(account::account_router(account::AccountState { db_path: db_path.clone(), cloud }))
        .merge(tv::tv_router(tv::TvState { db_path: db_path.clone() }))
        .merge(browse::browse_router(browse::BrowseState { db_path: db_path.clone() }))
        .merge(provider_status::provider_status_router(provider_status::ProviderStatusState::new(db_path.clone())))
        .merge(profiles::profiles_router(profiles::ProfilesState { db_path }))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    /// iOS (F15): si el servidor deja de responder (socket inservible tras suspender la
    /// app), `ensure` lo vuelve a abrir con el mismo router; si responde, no toca nada.
    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_reopens_a_server_that_stopped_answering() {
        let router = Router::new().route("/api/health", axum::routing::get(|| async { "ok" }));
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let control = ServerControl::new(port);
        control.start(listener, router).await.unwrap();
        assert!(responds(port).await);
        assert_eq!(control.ensure().await.unwrap(), port);

        // Simula el socket muerto: la tarea que sirve deja de existir.
        control.task.lock().await.take().unwrap().abort();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!responds(port).await);

        let new_port = control.ensure().await.unwrap();
        assert!(responds(new_port).await, "tras `ensure` el servidor tiene que responder");
    }

    #[tokio::test]
    async fn cors_covers_routes_merged_after_the_catalog_router() {
        let stream_state = stream::StreamState {
            db_path: std::env::temp_dir(),
            stream_guard: StreamGuard::new(),
            ffmpeg_path: None,
            ffprobe_path: None,
            mpv_path: None,
            last_errors: Default::default(),
            subtitles: Default::default(),
            timeshift: Default::default(),
            live_buffer: Default::default(),
        };
        let cloud: Arc<dyn CloudBackend> = Arc::new(crate::sync::memory::MemoryBackend::new());
        let router = full_router(std::env::temp_dir(), std::env::temp_dir(), stream_state, cloud);

        // Una respuesta a una ruta de F6 (fusionada tras el CORS de F4) debe llevar la
        // cabecera igual que una del catálogo: el bug real fue que no la llevaba.
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/playback/tier/movie/999999")
                    .header(header::ORIGIN, "http://localhost:1420")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.headers().contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN), "falta la cabecera CORS");
    }

    /// Verificación manual de punta a punta contra el Firebase real (F11): API HTTP local →
    /// cola de salida → motor en segundo plano → Firestore. Sobre una base de datos
    /// temporal, nunca la del usuario. Credenciales de una cuenta de pruebas por entorno:
    /// `VIBE_QA_EMAIL=... VIBE_QA_PASSWORD=... cargo test live_ -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_end_to_end_sync_through_the_local_api() {
        use crate::sync::backend::{Collection, RemoteDoc};
        use crate::sync::firebase::{FirebaseBackend, FirebaseConfig};
        use crate::sync::session;
        use http_body_util::BodyExt;

        let (Ok(email), Ok(password)) = (std::env::var("VIBE_QA_EMAIL"), std::env::var("VIBE_QA_PASSWORD")) else {
            panic!("faltan VIBE_QA_EMAIL / VIBE_QA_PASSWORD");
        };
        let _lock = crate::util::keychain::TEST_LOCK.lock().await;
        let started = crate::db::now_epoch();
        let movie_key = format!("live-e2e-{started}");
        let db_path = std::env::temp_dir().join(format!("vibe-iptv-live-e2e-{started}.sqlite3"));
        {
            let conn = schema::open_connection_at(&db_path).unwrap();
            conn.execute("INSERT INTO movies (id, clean_title, match_key) VALUES (1, 'Prueba E2E', ?1)", [&movie_key]).unwrap();
        }

        let cloud: Arc<dyn CloudBackend> = Arc::new(FirebaseBackend::new(FirebaseConfig::production()));
        engine::start(db_path.clone(), cloud.clone());
        let stream_state = stream::StreamState { db_path: db_path.clone(), stream_guard: StreamGuard::new(), ffmpeg_path: None, ffprobe_path: None, mpv_path: None, last_errors: Default::default(), subtitles: Default::default(), timeshift: Default::default(), live_buffer: Default::default() };
        let router = full_router(std::env::temp_dir(), db_path.clone(), stream_state, cloud.clone());

        let call = |method: &'static str, uri: String, body: serde_json::Value| {
            let router = router.clone();
            async move {
                let response = router
                    .oneshot(Request::builder().method(method).uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                assert!(status.is_success(), "{status}: {}", String::from_utf8_lossy(&bytes));
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or(serde_json::Value::Null)
            }
        };

        call("POST", "/api/account/login".into(), serde_json::json!({ "email": email, "password": password })).await;
        let profile = call("POST", "/api/profiles".into(), serde_json::json!({ "name": "E2E", "avatar": null, "adult_ok": false })).await;
        let profile_id = profile["id"].as_i64().unwrap();
        // La app nunca deja borrar el último perfil: un segundo permite limpiar el primero.
        call("POST", "/api/profiles".into(), serde_json::json!({ "name": "E2E bis", "avatar": null, "adult_ok": false })).await;
        call("POST", "/api/favorites".into(), serde_json::json!({ "profile_id": profile_id, "item_kind": "movie", "item_id": 1 })).await;
        call("POST", "/api/playback/movie-watched".into(), serde_json::json!({ "profile_id": profile_id, "movie_id": 1, "watched": true })).await;

        // El motor trabaja solo, en segundo plano: se espera a que vacíe la cola.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while engine::pending_changes(&db_path).await > 0 {
            assert!(std::time::Instant::now() < deadline, "la cola no se vació: {:?}", engine::status());
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let session = session::load().unwrap();
        let favorites = cloud.changes_since(&session, Collection::Favorites, started - 60).await.unwrap();
        assert!(favorites.docs.iter().any(|d| matches!(d, RemoteDoc::Favorite(f) if f.item_key == movie_key && !f.deleted)));
        let playback = cloud.changes_since(&session, Collection::Playback, started - 60).await.unwrap();
        assert!(playback.docs.iter().any(|d| matches!(d, RemoteDoc::Playback(p) if p.item_key == movie_key && p.finished)));
        let profiles = cloud.changes_since(&session, Collection::Profiles, started - 60).await.unwrap();
        assert!(profiles.docs.iter().any(|d| matches!(d, RemoteDoc::Profile(p) if p.name == "E2E")));

        // Limpieza: quitar el favorito y borrar el perfil deja borrados lógicos arriba. Por
        // pasos: al borrar un perfil, la cola descarta lo pendiente de ese perfil (los
        // demás dispositivos borran el perfil entero con su cascada).
        let wait_until_sent = |what: &'static str| {
            let db_path = db_path.clone();
            async move {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while engine::pending_changes(&db_path).await > 0 {
                    assert!(std::time::Instant::now() < deadline, "{what} no se envió: {:?}", engine::status());
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        };
        call("DELETE", format!("/api/favorites?profile_id={profile_id}&item_kind=movie&item_id=1"), serde_json::Value::Null).await;
        wait_until_sent("quitar el favorito").await;
        let favorites = cloud.changes_since(&session, Collection::Favorites, started - 60).await.unwrap();
        assert!(favorites.docs.iter().any(|d| matches!(d, RemoteDoc::Favorite(f) if f.item_key == movie_key && f.deleted)));

        let remote_profile_id: String = schema::open_connection_at(&db_path)
            .unwrap()
            .query_row("SELECT remote_id FROM profiles WHERE id = ?1", [profile_id], |r| r.get(0))
            .unwrap();
        call("DELETE", format!("/api/profiles/{profile_id}"), serde_json::Value::Null).await;
        wait_until_sent("borrar el perfil").await;
        let profiles = cloud.changes_since(&session, Collection::Profiles, started - 60).await.unwrap();
        assert!(profiles.docs.iter().any(|d| matches!(d, RemoteDoc::Profile(p) if p.id == remote_profile_id && p.deleted)));

        call("POST", "/api/account/logout".into(), serde_json::json!({})).await;
        assert!(session::load().is_none());
    }
}
