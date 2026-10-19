pub mod account;
pub mod api;
pub mod lan;
pub mod profiles;
pub mod routes;
pub mod stream;
pub mod timeshift;

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::db::schema;
use crate::sync::backend::CloudBackend;
use crate::sync::engine;
use crate::sync::firebase::{FirebaseBackend, FirebaseConfig};
use crate::util::pairing::PairingState;
use crate::util::stream_guard::StreamGuard;

/// Puerto del servidor local, gestionado como estado de Tauri para que el frontend lo
/// consulte con el comando `get_server_port`.
pub struct ServerPort(pub u16);

/// Comando Tauri que expone al frontend el puerto efímero en el que escucha el
/// servidor local (regla R2 de CLAUDE.md: nunca un puerto fijo).
#[tauri::command]
pub fn get_server_port(port: tauri::State<ServerPort>) -> u16 {
    port.0
}

/// Sirve la API local sobre un socket ya reservado en `127.0.0.1`. El puerto se decide
/// de forma síncrona en el arranque, antes de que el frontend pueda pedirlo.
pub async fn serve(
    listener: StdTcpListener,
    image_cache_dir: PathBuf,
    db_path: PathBuf,
    stream_guard: Arc<StreamGuard>,
) -> std::io::Result<()> {
    // En Windows, un socket duplicado con `try_clone()` no hereda el modo no bloqueante
    // del original (es un atributo de Winsock por handle, no por socket subyacente).
    // Sin esto, `accept()` bloquea el hilo del runtime y el servidor deja de responder
    // sin ningún error visible.
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;

    // Aplica las migraciones aquí, antes de servir nada. Sin esto había una carrera de
    // verdad, no solo teórica: en una instalación nueva (fichero de base de datos
    // inexistente) la primera petición HTTP real llega por `open_readonly_at` (que
    // nunca comprueba migraciones) y podría adelantarse al primer comando Tauri que sí
    // las dispara (`list_providers`, vía `open_connection`). De paso, es el sitio
    // natural para leer el ajuste de LAN antes de construir su estado.
    let lan_enabled = {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || -> crate::db::Result<bool> {
            let conn = schema::open_connection_at(&db_path)?;
            lan::read_lan_enabled_setting(&conn)
        })
        .await
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other)?
    };
    let lan_state = lan::LanState { db_path: db_path.clone(), enabled: Arc::new(AtomicBool::new(lan_enabled)), pairing: PairingState::new() };

    crate::media::subtitles::clear_temp_dir();
    timeshift::clear_temp_root();
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

    let router = full_router(image_cache_dir, db_path, stream_state, lan_state, cloud);
    // `with_connect_info` es lo que le da a `lan::require_local_or_paired` la IP real
    // del que llama — sin esto no habría forma de distinguir loopback de LAN.
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await
}

/// El router completo: catálogo + salud/imágenes + streaming, con CORS aplicado una
/// sola vez sobre el conjunto. Separado de `serve()` (que además reserva el socket real)
/// para poder probarlo con `oneshot` sin abrir ningún puerto.
///
/// CORS abierto a cualquier origen: el WebView de Tauri sirve el frontend desde un
/// origen distinto al del servidor (`http://localhost:1420` en dev, `tauri://localhost`
/// empaquetado), y este servidor solo escucha en 127.0.0.1 sin cookies ni sesión (R2).
/// Se revisará al abrir la LAN en la fase F10.
///
/// Tiene que envolver el router ya fusionado, no una de sus mitades por separado: un
/// `.layer()` aplicado antes de `.merge()` no cubre las rutas que se fusionan después.
/// Ese error real llegó a producción (F6): `fetch()` desde el WebView a
/// `/api/playback/tier/*` fallaba en silencio por CORS —`<video src>` no lo notó porque
/// cargar un recurso de vídeo no pasa por CORS, solo `fetch`/`XHR` sí— y el panel de
/// estadísticas del reproductor se quedó mostrando "nivel desconocido" para siempre.
///
/// El middleware del modo LAN (F10) se aplica antes del CORS (queda por dentro): así
/// una petición de preflight (`OPTIONS`, sin `Authorization`) la resuelve `CorsLayer`
/// sin pasar nunca por la comprobación de token, que no tiene sentido aplicarle.
fn full_router(
    image_cache_dir: PathBuf,
    db_path: PathBuf,
    stream_state: stream::StreamState,
    lan_state: lan::LanState,
    cloud: Arc<dyn CloudBackend>,
) -> Router {
    routes::build_router(image_cache_dir, db_path.clone())
        .merge(stream::stream_router(stream_state))
        .merge(lan::lan_router(lan_state.clone()))
        .merge(account::account_router(account::AccountState { db_path: db_path.clone(), cloud }))
        .merge(profiles::profiles_router(profiles::ProfilesState { db_path }))
        .layer(axum::middleware::from_fn_with_state(lan_state, lan::require_local_or_paired))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

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
        };
        let lan_state = lan::LanState {
            db_path: std::env::temp_dir(),
            enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pairing: crate::util::pairing::PairingState::new(),
        };
        let cloud: Arc<dyn CloudBackend> = Arc::new(crate::sync::memory::MemoryBackend::new());
        let router = full_router(std::env::temp_dir(), std::env::temp_dir(), stream_state, lan_state, cloud);

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
        let stream_state = stream::StreamState { db_path: db_path.clone(), stream_guard: StreamGuard::new(), ffmpeg_path: None, ffprobe_path: None, mpv_path: None, last_errors: Default::default(), subtitles: Default::default(), timeshift: Default::default() };
        let lan_state = lan::LanState { db_path: db_path.clone(), enabled: Arc::new(AtomicBool::new(false)), pairing: PairingState::new() };
        let router = full_router(std::env::temp_dir(), db_path.clone(), stream_state, lan_state, cloud.clone());

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
