//! `GET /stream/:kind/:id` — la cascada de reproducción (F6, ver
//! `docs/ARCHITECTURE.md`). Todo el vídeo pasa por aquí: la API de catálogo (F4) nunca
//! devuelve una URL de proveedor (regla R5); el reproductor solo conoce esta ruta local.
//!
//! `:kind` es `channel`, `movie` o `episode`. `?source=<id>` elige una fuente concreta de
//! una película con varias calidades; `?start=<segundos>` pide un remux (nivel 2)
//! empezando en ese punto — así implementa la búsqueda en VOD (ver `spawn_remux_response`).

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{BoxError, Router};
use futures_util::StreamExt;
use reqwest::Url;
use serde::Deserialize;
use tokio_util::io::ReaderStream;

use crate::commands::build_source_for_provider;
use crate::db::{queries, reads, schema, DbError};
use crate::media::{ffmpeg, probe};
use crate::sources::model::{MediaKind, StreamRef};
use crate::sources::xtream::DEFAULT_USER_AGENT;
use crate::sources::Source;
use crate::util::redact;
use crate::util::stream_guard::{StreamGuard, StreamPermit};

#[derive(Clone)]
pub struct StreamState {
    pub db_path: PathBuf,
    pub stream_guard: Arc<StreamGuard>,
    /// `None` si `media::resolve_binary` no encontró el binario: la ruta responde 503 en
    /// vez de entrar en pánico. En v1 se busca en el `PATH` (ver `media::resolve_binary`);
    /// F10 lo sustituye por la ruta del sidecar empaquetado.
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
}

pub fn stream_router(state: StreamState) -> Router {
    Router::new()
        .route("/stream/:kind/:id", get(get_stream))
        .route("/api/playback/tier/:kind/:id", get(get_playback_tier))
        .with_state(state)
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct PlaybackTierResponse {
    /// `None` si el ítem nunca se ha reproducido todavía: la sonda solo ocurre al abrir
    /// el stream de verdad (regla R1), así que hasta entonces no hay nada que enseñar.
    #[ts(type = "number | null")]
    pub tier: Option<u8>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    /// Duración real del fichero en segundos (VOD), tal cual la dio ffprobe. `None` en
    /// directo o si todavía no se ha reproducido nunca este ítem.
    pub duration_s: Option<f64>,
}

/// Panel de estadísticas del reproductor (Ctrl+Shift+D, F6): lee `probe_cache`, nunca
/// abre una conexión — resolver la URL del ítem y calcular su hash es puro cálculo local,
/// sin red, así que no cuenta como el prefetch que prohíbe R1.
async fn get_playback_tier(State(state): State<StreamState>, AxumPath((kind, id)): AxumPath<(String, i64)>) -> Result<axum::Json<PlaybackTierResponse>, StreamError> {
    let media_kind = parse_kind(&kind)?;
    let (provider_id, remote_id, container_extension) = resolve_stream_target(&state.db_path, media_kind, id, None).await?;
    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;
    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;
    let item = StreamRef { kind: media_kind, remote_id, container_extension };
    let url = source.stream_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    let cache_key = probe::stream_cache_key(url.as_str());
    let cached = with_db(state.db_path, move |conn| queries::get_probe_cache(conn, &cache_key)).await?;

    Ok(axum::Json(match cached {
        Some(row) => PlaybackTierResponse {
            tier: Some(row.play_tier as u8),
            video_codec: row.video_codec,
            audio_codec: row.audio_codec,
            container: row.container,
            duration_s: row.duration_s,
        },
        None => PlaybackTierResponse { tier: None, video_codec: None, audio_codec: None, container: None, duration_s: None },
    }))
}

#[derive(Debug, thiserror::Error)]
enum StreamError {
    #[error("no encontrado")]
    NotFound,
    #[error("kind inválido: debe ser channel, movie o episode")]
    BadKind,
    #[error("proveedor no encontrado")]
    ProviderNotFound,
    #[error("no se pudo preparar la fuente: {0}")]
    Source(String),
    #[error("no se pudo abrir el stream del proveedor")]
    Unreachable,
    #[error("ffmpeg o ffprobe no están disponibles todavía (se instalan como sidecar en F10)")]
    BinariesMissing,
    #[error("este contenido necesita el nivel 3 (mpv), disponible en la fase F9")]
    Tier3NotYetImplemented,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl IntoResponse for StreamError {
    fn into_response(self) -> Response {
        let status = match &self {
            StreamError::NotFound | StreamError::ProviderNotFound => StatusCode::NOT_FOUND,
            StreamError::BadKind => StatusCode::BAD_REQUEST,
            StreamError::Source(_) | StreamError::Unreachable => StatusCode::BAD_GATEWAY,
            StreamError::BinariesMissing => StatusCode::SERVICE_UNAVAILABLE,
            StreamError::Tier3NotYetImplemented => StatusCode::NOT_IMPLEMENTED,
            StreamError::Db(_) | StreamError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

async fn with_db<T, F>(db_path: PathBuf, f: F) -> Result<T, StreamError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::db::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_readonly_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|_| StreamError::Internal)?
    .map_err(StreamError::Db)
}

#[derive(Deserialize)]
struct StreamQuery {
    source: Option<i64>,
    start: Option<f64>,
}

async fn get_stream(
    State(state): State<StreamState>,
    AxumPath((kind, id)): AxumPath<(String, i64)>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let range = headers.get(header::RANGE).cloned();
    match handle_stream(state, kind.clone(), id, query.source, query.start, range).await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(kind = %kind, id, error = %err, "fallo al servir /stream");
            err.into_response()
        }
    }
}

fn parse_kind(kind: &str) -> Result<MediaKind, StreamError> {
    match kind {
        "channel" => Ok(MediaKind::Live),
        "movie" => Ok(MediaKind::Movie),
        "episode" => Ok(MediaKind::Series),
        _ => Err(StreamError::BadKind),
    }
}

async fn resolve_stream_target(
    db_path: &PathBuf,
    media_kind: MediaKind,
    id: i64,
    source_id: Option<i64>,
) -> Result<(i64, String, Option<String>), StreamError> {
    let target = with_db(db_path.clone(), move |conn| match media_kind {
        MediaKind::Live => Ok(reads::resolve_channel_stream(conn, id)?.map(|c| (c.provider_id, c.remote_id, None))),
        MediaKind::Movie => {
            Ok(reads::resolve_movie_stream(conn, id, source_id)?.map(|m| (m.provider_id, m.remote_id, m.container_extension)))
        }
        MediaKind::Series => {
            Ok(reads::resolve_episode_stream(conn, id)?.map(|e| (e.provider_id, e.remote_id, e.container_extension)))
        }
    })
    .await?;
    target.ok_or(StreamError::NotFound)
}

/// Registra qué nivel sirvió el ítem (diagnóstico, ver `docs/ARCHITECTURE.md`). No es
/// fatal si falla: no debe tumbar la reproducción, que ya está en marcha.
async fn record_tier(db_path: &PathBuf, item_kind: &str, item_id: i64, tier: u8) {
    let db_path = db_path.clone();
    let item_kind = item_kind.to_string();
    let outcome = tokio::task::spawn_blocking(move || -> crate::db::Result<()> {
        let conn = schema::open_readonly_at(&db_path)?;
        let profile_id = reads::ensure_default_profile(&conn)?;
        queries::record_play_tier(&conn, profile_id, &item_kind, item_id, tier as i64)
    })
    .await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!(error = %err, "no se pudo registrar el nivel de reproducción"),
        Err(err) => tracing::warn!(error = %err, "la tarea de registro de nivel de reproducción falló"),
    }
}

async fn handle_stream(
    state: StreamState,
    kind_str: String,
    id: i64,
    source_id: Option<i64>,
    start_seconds: Option<f64>,
    range_header: Option<HeaderValue>,
) -> Result<Response, StreamError> {
    let media_kind = parse_kind(&kind_str)?;
    let (provider_id, remote_id, container_extension) = resolve_stream_target(&state.db_path, media_kind, id, source_id).await?;

    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;
    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;

    let item = StreamRef { kind: media_kind, remote_id, container_extension };
    let primary_url = source.stream_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    // A partir de aquí se abre (o se reutiliza) LA conexión de stream: cierra la
    // anterior y espera su muerte real antes de devolver el permiso (regla R1).
    let permit = state.stream_guard.acquire().await;

    // Nivel 1a: HLS reconocido por la extensión, sin necesidad de sonda. No reescribe
    // las URIs internas del manifiesto (ver docs/DECISIONS.md): no se ha podido probar
    // contra un proveedor que sirva HLS de verdad, ninguno de los dos del usuario lo hace.
    if primary_url.path().to_ascii_lowercase().ends_with(".m3u8") {
        record_tier(&state.db_path, &kind_str, id, 1).await;
        return proxy_passthrough(primary_url, range_header, permit, Arc::clone(&state.stream_guard)).await;
    }

    let (ffmpeg_path, ffprobe_path) = match (&state.ffmpeg_path, &state.ffprobe_path) {
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg.clone(), ffprobe.clone()),
        _ => return Err(StreamError::BinariesMissing),
    };

    let (resolved_url, tier, audio_codec) =
        resolve_tier_and_url(&source, &item, primary_url, &ffprobe_path, &state.db_path).await?;
    record_tier(&state.db_path, &kind_str, id, tier).await;

    match tier {
        1 => proxy_passthrough(resolved_url, range_header, permit, Arc::clone(&state.stream_guard)).await,
        2 => {
            let transcode_audio = probe::audio_needs_transcode(audio_codec.as_deref());
            spawn_remux_response(&ffmpeg_path, resolved_url.as_str(), transcode_audio, start_seconds, permit, Arc::clone(&state.stream_guard))
                .await
        }
        _ => Err(StreamError::Tier3NotYetImplemented),
    }
}

/// Decide el nivel y la URL a usar. Si la sonda ya está cacheada (por ítem, identificada
/// por el hash de la URL principal), no vuelve a abrir ninguna conexión. Si no, sondea; si
/// la URL principal falla y la fuente ofrece una alternativa (paneles Xtream antiguos sin
/// `/live/`, ver `Source::stream_url_fallback`), reintenta con ella antes de rendirse.
async fn resolve_tier_and_url(
    source: &Arc<dyn Source + Send + Sync>,
    item: &StreamRef,
    primary_url: Url,
    ffprobe_path: &Path,
    db_path: &PathBuf,
) -> Result<(Url, u8, Option<String>), StreamError> {
    let cache_key = probe::stream_cache_key(primary_url.as_str());
    let cached = with_db(db_path.clone(), {
        let cache_key = cache_key.clone();
        move |conn| queries::get_probe_cache(conn, &cache_key)
    })
    .await?;
    if let Some(cached) = cached {
        return Ok((primary_url, cached.play_tier as u8, cached.audio_codec));
    }

    let probe_attempt = probe::probe_url(ffprobe_path, primary_url.as_str(), DEFAULT_USER_AGENT).await;
    let (resolved_url, probe_result) = match probe_attempt {
        Ok(result) => (primary_url, result),
        Err(primary_err) => {
            let fallback = source.stream_url_fallback(item).map_err(|err| StreamError::Source(err.to_string()))?;
            let Some(fallback_url) = fallback else {
                tracing::warn!(error = %primary_err, "no se pudo inspeccionar el stream y no hay URL alternativa");
                return Err(StreamError::Unreachable);
            };
            match probe::probe_url(ffprobe_path, fallback_url.as_str(), DEFAULT_USER_AGENT).await {
                Ok(result) => (fallback_url, result),
                Err(fallback_err) => {
                    tracing::warn!(primary = %primary_err, fallback = %fallback_err, "ninguna de las dos formas de la URL respondió");
                    return Err(StreamError::Unreachable);
                }
            }
        }
    };

    let tier = probe::decide_tier(&probe_result.container, probe_result.video_codec.as_deref(), probe_result.audio_codec.as_deref());
    // La duración que da ffprobe en directo es la del trozo que llegó a analizar, no la
    // del canal (no tiene fin): solo tiene sentido guardarla en VOD.
    let duration_s = if item.kind == MediaKind::Live { None } else { probe_result.duration_seconds };
    let store_key = probe::stream_cache_key(resolved_url.as_str());
    let video = probe_result.video_codec.clone();
    let audio = probe_result.audio_codec.clone();
    let container = probe_result.container.clone();
    with_db(db_path.clone(), move |conn| {
        queries::set_probe_cache(conn, &store_key, video.as_deref(), audio.as_deref(), Some(&container), tier as i64, duration_s)
    })
    .await?;

    Ok((resolved_url, tier, probe_result.audio_codec))
}

type BoxedByteStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, BoxError>> + Send>>;

/// Envuelve el cuerpo de la respuesta para que el permiso de `StreamGuard` (y, si lo
/// hay, el proceso ffmpeg de nivel 2) se liberen quando el cuerpo se suelta — tanto si la
/// reproducción termina normal como si el cliente corta la conexión: axum suelta el
/// `Body` en los dos casos, así que un único `Drop` cubre ambos.
struct PermitBoundBody {
    inner: BoxedByteStream,
    permit: Option<StreamPermit>,
    guard: Arc<StreamGuard>,
    generation: u64,
}

impl futures_core::Stream for PermitBoundBody {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for PermitBoundBody {
    fn drop(&mut self) {
        let guard = Arc::clone(&self.guard);
        let generation = self.generation;
        // Matar el proceso ffmpeg (si lo hay) necesita `.await`; `Drop` no puede
        // ofrecerlo, así que se termina en una tarea aparte. Si esta sesión no es ya la
        // vigente (`acquire()` ya se llevó el proceso), `release_process` no hace nada.
        tokio::spawn(async move {
            guard.release_process(generation).await;
        });
        drop(self.permit.take()); // libera `StreamGuard::is_busy()` para esta sesión.
    }
}

/// Nivel 1: proxy directo con soporte de Range, sin tocar los bytes. Vale tanto para un
/// MP4 progresivo (H.264/AAC) como, de forma básica, para un manifiesto HLS (ver el aviso
/// de `handle_stream` sobre reescritura de URIs).
async fn proxy_passthrough(url: Url, range: Option<HeaderValue>, permit: StreamPermit, guard: Arc<StreamGuard>) -> Result<Response, StreamError> {
    let client = reqwest::Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .build()
        .map_err(|_| StreamError::Internal)?;

    let mut request = client.get(url);
    if let Some(range) = &range {
        if let Ok(value) = range.to_str() {
            request = request.header(header::RANGE, value);
        }
    }

    let upstream = request.send().await.map_err(|err| {
        tracing::warn!(error = %redact(&err.to_string()), "no se pudo conectar con el proveedor para el proxy directo");
        StreamError::Unreachable
    })?;

    let status = upstream.status();
    if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        return Err(StreamError::Unreachable);
    }

    let mut builder = Response::builder().status(status);
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE, header::ACCEPT_RANGES] {
        if let Some(value) = upstream.headers().get(&name) {
            builder = builder.header(name, value.clone());
        }
    }

    let generation = permit.generation();
    let inner: BoxedByteStream = Box::pin(upstream.bytes_stream().map(|item| item.map_err(|err| Box::new(err) as BoxError)));
    let body = Body::from_stream(PermitBoundBody { inner, permit: Some(permit), guard, generation });
    builder.body(body).map_err(|_| StreamError::Internal)
}

/// Nivel 2: remux a fMP4 con ffmpeg, un proceso vivo por sesión (ver
/// `util::stream_guard`). `start_seconds` implementa la búsqueda en VOD: el cliente pide
/// un stream nuevo con `?start=<segundos>` y aquí se traduce a un `-ss` de entrada.
async fn spawn_remux_response(
    ffmpeg_path: &Path,
    url: &str,
    transcode_audio: bool,
    start_seconds: Option<f64>,
    permit: StreamPermit,
    guard: Arc<StreamGuard>,
) -> Result<Response, StreamError> {
    let mut child = ffmpeg::spawn_remux(ffmpeg_path, url, DEFAULT_USER_AGENT, transcode_audio, start_seconds).map_err(|err| {
        tracing::error!(error = %err, "no se pudo lanzar ffmpeg para el remux");
        StreamError::BinariesMissing
    })?;
    let stdout = child.stdout.take().ok_or(StreamError::Internal)?;

    guard.attach_process(&permit, child).await;

    let generation = permit.generation();
    let inner: BoxedByteStream = Box::pin(ReaderStream::new(stdout).map(|item| item.map_err(|err| Box::new(err) as BoxError)));
    let body = Body::from_stream(PermitBoundBody { inner, permit: Some(permit), guard, generation });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .body(body)
        .map_err(|_| StreamError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;
    use tower_http::services::ServeFile;

    use crate::db::queries::{self as write_queries, ProviderKind, TitleFields};
    use crate::db::schema::open_connection_at;
    use crate::sources::model::{RawChannel, RawMovie};
    use crate::util::keychain;

    fn which_on_path(name: &str) -> Option<PathBuf> {
        crate::media::resolve_binary(name)
    }

    /// Borra el secreto de prueba del keychain real al terminar, pase lo que pase (igual
    /// que `util::keychain`'s propios tests: no hay backend de prueba, es el almacén de
    /// credenciales real del sistema).
    struct KeychainGuard(String);
    impl Drop for KeychainGuard {
        fn drop(&mut self) {
            let _ = keychain::delete_secret(&self.0);
        }
    }

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vibe-iptv-stream-test-{label}-{}-{}.sqlite3", std::process::id(), rand_suffix()))
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    async fn generate_sample(ffmpeg_path: &Path, dir: &Path, filename: &str, container: &str) -> PathBuf {
        let out = dir.join(filename);
        let status = tokio::process::Command::new(ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=64x64:rate=10", "-f", "lavfi", "-i", "sine=duration=2"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-t", "2", "-f", container])
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success(), "no se pudo generar el fichero de prueba {filename}");
        out
    }

    /// Sirve `file` en `route_path` (la forma exacta `/movie/user/pass/1.mp4` que
    /// construiría `XtreamSource`), con soporte de Range real vía `tower_http::ServeFile`
    /// — así la prueba ejercita el mismo comportamiento que un proveedor real.
    async fn spawn_file_server(route_path: &str, file: PathBuf) -> String {
        let router = Router::new().route_service(route_path, ServeFile::new(file));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    fn seed_provider_with_credential(db_path: &Path, base_url: &str, credential_suffix: &str) -> (i64, KeychainGuard) {
        let conn = open_connection_at(db_path).unwrap();
        let provider_id =
            write_queries::insert_provider(&conn, ProviderKind::Xtream, "P", base_url, Some("user"), 1, None).unwrap();
        let credential_ref = format!("vibe-iptv-stream-test-{}-{credential_suffix}", std::process::id());
        keychain::store_secret(&credential_ref, "pass").unwrap();
        write_queries::set_provider_credential_ref(&conn, provider_id, &credential_ref).unwrap();
        (provider_id, KeychainGuard(credential_ref))
    }

    fn base_state(db_path: PathBuf, ffmpeg_path: Option<PathBuf>, ffprobe_path: Option<PathBuf>) -> StreamState {
        StreamState { db_path, stream_guard: StreamGuard::new(), ffmpeg_path, ffprobe_path }
    }

    #[tokio::test]
    async fn returns_400_for_an_invalid_kind() {
        let db_path = temp_db_path("badkind");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone(), None, None);

        let response = stream_router(state)
            .oneshot(Request::builder().uri("/stream/podcast/1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn returns_404_for_an_unknown_channel() {
        let db_path = temp_db_path("notfound");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone(), None, None);

        let response = stream_router(state)
            .oneshot(Request::builder().uri("/stream/channel/9999").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn tier_one_proxies_a_progressive_mp4_with_range_support_and_caches_the_probe() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = generate_sample(&ffmpeg_path, &dir, "sample.mp4", "mp4").await;
        let base_url = spawn_file_server("/movie/user/pass/1.mp4", sample).await;

        let db_path = temp_db_path("mp4");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "mp4");
        {
            let conn = open_connection_at(&db_path).unwrap();
            let raw = RawMovie {
                remote_id: "1".into(),
                category_remote_id: None,
                raw_title: "Peli".into(),
                container_extension: Some("mp4".into()),
                logo_url: None,
                added_at: None,
                rating: None,
                tmdb_id: None,
            };
            let fields = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-2020", quality: None, language: None };
            write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
        }
        let movie_id: i64 = {
            let conn = open_connection_at(&db_path).unwrap();
            conn.query_row("SELECT id FROM movies WHERE match_key = 'peli-2020'", [], |r| r.get(0)).unwrap()
        };

        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path));
        let response = stream_router(state)
            .oneshot(Request::builder().uri(format!("/stream/movie/{movie_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key(header::ACCEPT_RANGES));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.is_empty());
        assert_eq!(&body[4..8], b"ftyp");

        let conn = open_connection_at(&db_path).unwrap();
        let (tier, duration): (i64, Option<f64>) =
            conn.query_row("SELECT play_tier, duration_s FROM probe_cache", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(tier, 1, "un MP4 progresivo H.264/AAC debe resolverse a nivel 1");
        // La duración real (VOD) se guarda para que el reproductor pueda pintar una
        // barra de búsqueda aunque TMDB no tenga la duración editorial del título.
        assert!(duration.is_some_and(|d| d > 1.5 && d < 3.0), "duración inesperada: {duration:?}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn tier_two_remuxes_a_live_ts_channel_to_fragmented_mp4() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-ts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = generate_sample(&ffmpeg_path, &dir, "sample.ts", "mpegts").await;
        let base_url = spawn_file_server("/live/user/pass/1.ts", sample).await;

        let db_path = temp_db_path("ts");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "ts");
        {
            let conn = open_connection_at(&db_path).unwrap();
            let channel = RawChannel {
                remote_id: "1".into(),
                category_remote_id: None,
                raw_name: "Canal".into(),
                logo_url: None,
                epg_channel_id: None,
                has_catchup: false,
                catchup_days: 0,
                sort_order: None,
            };
            let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
            write_queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();
        }
        let channel_id: i64 = {
            let conn = open_connection_at(&db_path).unwrap();
            conn.query_row("SELECT id FROM channels WHERE remote_id = '1'", [], |r| r.get(0)).unwrap()
        };

        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path));
        let response = stream_router(state)
            .oneshot(Request::builder().uri(format!("/stream/channel/{channel_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::CONTENT_TYPE).unwrap(), "video/mp4");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.is_empty());
        assert_eq!(&body[4..8], b"ftyp");

        let conn = open_connection_at(&db_path).unwrap();
        let (tier, duration): (i64, Option<f64>) =
            conn.query_row("SELECT play_tier, duration_s FROM probe_cache", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(tier, 2, "MPEG-TS siempre necesita remux, nunca es nivel 1");
        // La "duración" que ve ffprobe en directo es la del trozo analizado, no la del
        // canal (no tiene fin): no debe guardarse, o el reproductor pintaría una barra
        // de búsqueda falsa en un canal en directo.
        assert_eq!(duration, None, "un canal en directo no debe guardar duración");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&db_path);
    }

    /// Verificación manual del criterio de aceptación de F6: "cambiar de canal veinte
    /// veces seguidas no deja procesos ffmpeg huérfanos". Cuenta procesos `ffmpeg.exe`
    /// del sistema (vía `tasklist`), no solo del propio proceso de test, así que es
    /// frágil si la máquina tiene otros ffmpeg corriendo — por eso se ignora por
    /// defecto y se ejecuta a mano una vez, documentando el resultado en
    /// docs/DECISIONS.md, en vez de vivir en la batería de tests normal.
    #[tokio::test]
    #[ignore]
    async fn twenty_rapid_channel_switches_leave_no_orphaned_ffmpeg_processes() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        fn count_ffmpeg_processes() -> usize {
            let output = std::process::Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq ffmpeg.exe", "/FO", "CSV", "/NH"])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines().filter(|line| line.contains("ffmpeg.exe")).count()
        }

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-zap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = generate_sample(&ffmpeg_path, &dir, "sample.ts", "mpegts").await;
        let base_url = spawn_file_server("/live/user/pass/1.ts", sample).await;

        let db_path = temp_db_path("zap");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "zap");
        {
            let conn = open_connection_at(&db_path).unwrap();
            let channel = RawChannel {
                remote_id: "1".into(),
                category_remote_id: None,
                raw_name: "Canal".into(),
                logo_url: None,
                epg_channel_id: None,
                has_catchup: false,
                catchup_days: 0,
                sort_order: None,
            };
            let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
            write_queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();
        }
        let channel_id: i64 = {
            let conn = open_connection_at(&db_path).unwrap();
            conn.query_row("SELECT id FROM channels WHERE remote_id = '1'", [], |r| r.get(0)).unwrap()
        };

        let baseline = count_ffmpeg_processes();
        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path));
        let router = stream_router(state);

        for i in 0..20 {
            let response = router
                .clone()
                .oneshot(Request::builder().uri(format!("/stream/channel/{channel_id}")).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "cambio de canal {i} falló");
            // No se lee el cuerpo: simula cambiar de canal antes de que termine de
            // cargar, que es exactamente el caso que puede dejar procesos huérfanos.
            drop(response);
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }

        // Un margen para que la última tarea de limpieza en segundo plano termine.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let after = count_ffmpeg_processes();
        assert!(
            after <= baseline + 1,
            "quedaron procesos ffmpeg huérfanos: base={baseline}, después de 20 cambios={after}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&db_path);
    }
}
