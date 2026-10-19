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
use crate::media::ffmpeg::RemuxOptions;
use crate::media::{ffmpeg, mpv, probe, subtitles};
use crate::sources::model::{CatchupRef, MediaKind, StreamRef};
use crate::sources::xtream::DEFAULT_USER_AGENT;
use crate::sources::Source;
use crate::util::redact;
use crate::util::stream_guard::{StreamGuard, StreamPermit};

use super::timeshift::{self, TimeshiftSession, TimeshiftSlot};

#[derive(Clone)]
pub struct StreamState {
    pub db_path: PathBuf,
    pub stream_guard: Arc<StreamGuard>,
    /// `None` si `media::resolve_binary` no encontró el binario: la ruta responde 503 en
    /// vez de entrar en pánico. En v1 se busca en el `PATH` (ver `media::resolve_binary`);
    /// F10 lo sustituye por la ruta del sidecar empaquetado.
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    /// Nivel 3 (F9): igual de opcional y por el mismo motivo. Sin verificar de verdad
    /// (ver `media::mpv`) — no hay mpv instalado en la máquina donde se escribió esto.
    pub mpv_path: Option<PathBuf>,
    /// Último fallo al abrir cada stream, por `"<kind>:<id>"`. El `<video>` nunca ve el
    /// cuerpo de una respuesta de error; el reproductor lo consulta aquí para explicar qué
    /// pasó, sin volver a pedir el stream (que abriría otra sesión con el proveedor).
    pub last_errors: StreamErrorLog,
    /// Subtítulos que va dejando el remux en curso (F12), si se pidió alguna pista.
    pub subtitles: SubtitleSession,
    /// Grabación del directo en curso, para pausar y rebobinar (F13).
    pub timeshift: TimeshiftSlot,
}

pub type StreamErrorLog = Arc<std::sync::Mutex<std::collections::HashMap<String, StreamFailure>>>;

pub type SubtitleSession = Arc<std::sync::Mutex<Option<ActiveSubtitles>>>;

/// La pista de subtítulos de la sesión de remux vigente. Solo hay una a la vez, como
/// solo hay un stream a la vez (regla R1).
pub struct ActiveSubtitles {
    /// Ver `subtitle_session_key`.
    key: String,
    path: PathBuf,
}

/// Identifica una sesión de subtítulos: ítem, pista y punto de arranque. Con el punto de
/// arranque, un cliente que acaba de buscar no puede leer por error las frases del remux
/// anterior (otra escala de tiempos) en el instante antes de que arranque el nuevo.
fn subtitle_session_key(kind: &str, id: i64, track: i64, start_seconds: Option<f64>) -> String {
    format!("{kind}:{id}:{track}:{}", start_seconds.unwrap_or(0.0))
}

/// Cierra la sesión de subtítulos anterior (borra su fichero) y, si se pide, abre otra.
/// Se llama ya con el permiso de `StreamGuard`, es decir, con el ffmpeg anterior muerto.
fn replace_subtitle_session(session: &SubtitleSession, key: Option<String>) -> Option<PathBuf> {
    let Ok(mut current) = session.lock() else { return None };
    if let Some(previous) = current.take() {
        let _ = std::fs::remove_file(&previous.path);
    }
    let key = key?;
    match subtitles::new_temp_file() {
        Ok(path) => {
            *current = Some(ActiveSubtitles { key, path: path.clone() });
            Some(path)
        }
        Err(err) => {
            tracing::warn!(error = %err, "no se pudo preparar el fichero de subtítulos; se reproduce sin ellos");
            None
        }
    }
}

#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct StreamFailure {
    pub status: u16,
    pub message: String,
}

fn remember_outcome(log: &StreamErrorLog, key: String, outcome: Result<&Response, &StreamError>) {
    let Ok(mut map) = log.lock() else { return };
    match outcome {
        Ok(_) => {
            map.remove(&key);
        }
        Err(err) => {
            map.insert(key, StreamFailure { status: err.status().as_u16(), message: err.to_string() });
        }
    }
}

pub fn stream_router(state: StreamState) -> Router {
    Router::new()
        .route("/stream/:kind/:id", get(get_stream))
        .route("/stream/mpv/:kind/:id", axum::routing::post(post_launch_mpv))
        .route("/stream/catchup/:channel_id", get(get_catchup))
        .route("/api/playback/tier/:kind/:id", get(get_playback_tier))
        .route("/api/playback/binaries", get(get_binaries_status))
        .route("/api/playback/last-error/:kind/:id", get(get_last_error))
        .route("/api/playback/subtitles/:kind/:id", get(get_subtitle_cues))
        .route("/stream/timeshift/channel/:id/:file", get(timeshift::get_file))
        .route("/api/timeshift/:channel_id/keepalive", axum::routing::post(timeshift::post_keepalive))
        .route("/api/timeshift/:channel_id", axum::routing::delete(timeshift::delete_session))
        .with_state(state)
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BinariesStatus {
    pub ffmpeg: bool,
    pub ffprobe: bool,
    pub mpv: bool,
}

/// "Comprobación de que existen al arrancar" (F10): el arranque ya deja constancia en el
/// log (`tracing::warn!` en `server::serve`), pero eso no lo ve nadie en el día a día —
/// esto le da a Ajustes algo que consultar para enseñar un aviso claro si faltase algún
/// sidecar, en vez de que el usuario se entere solo cuando intente reproducir algo.
async fn get_binaries_status(State(state): State<StreamState>) -> axum::Json<BinariesStatus> {
    axum::Json(BinariesStatus {
        ffmpeg: state.ffmpeg_path.is_some(),
        ffprobe: state.ffprobe_path.is_some(),
        mpv: state.mpv_path.is_some(),
    })
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
    /// Pistas de audio del stream (F "premium" — selector de pista de audio). Vacío si
    /// nunca se ha reproducido, o si solo tiene una (no hace falta selector para eso).
    pub audio_tracks: Vec<AudioTrackSummary>,
    /// Pistas de subtítulos que se pueden enseñar (solo las de texto: las de imagen, como
    /// PGS, no se pueden pasar a WebVTT). Vacío si nunca se ha reproducido.
    pub subtitle_tracks: Vec<SubtitleTrackSummary>,
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SubtitleTrackSummary {
    #[ts(type = "number")]
    pub index: i64,
    pub language: Option<String>,
    pub codec_name: Option<String>,
}

/// Copia con derives de ts-rs de `media::probe::AudioTrack` — ese tipo vive en `media`,
/// que no conoce nada de la API HTTP, mismo patrón que el resto de tipos de respuesta de
/// este fichero envolviendo los de `db::reads`.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct AudioTrackSummary {
    #[ts(type = "number")]
    pub index: i64,
    pub language: Option<String>,
    pub codec_name: Option<String>,
    #[ts(type = "number | null")]
    pub channels: Option<i64>,
}

impl From<probe::AudioTrack> for AudioTrackSummary {
    fn from(t: probe::AudioTrack) -> Self {
        Self { index: t.index, language: t.language, codec_name: t.codec_name, channels: t.channels }
    }
}

/// Panel de estadísticas del reproductor (Ctrl+Shift+D, F6): lee `probe_cache`, nunca
/// abre una conexión — resolver la URL del ítem y calcular su hash es puro cálculo local,
/// sin red, así que no cuenta como el prefetch que prohíbe R1.
#[derive(Deserialize)]
struct TierQuery {
    /// Igual que en `StreamQuery`: sin esto, la ficha de una película con selector de
    /// fuente (F "premium") vería siempre el nivel/códecs de la fuente por defecto,
    /// aunque se estuviera reproduciendo otra distinta.
    source: Option<i64>,
}

async fn get_playback_tier(
    State(state): State<StreamState>,
    AxumPath((kind, id)): AxumPath<(String, i64)>,
    Query(query): Query<TierQuery>,
) -> Result<axum::Json<PlaybackTierResponse>, StreamError> {
    let media_kind = parse_kind(&kind)?;
    let (provider_id, remote_id, container_extension) = resolve_stream_target(&state.db_path, media_kind, id, query.source).await?;
    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;
    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;
    let item = StreamRef { kind: media_kind, remote_id, container_extension };
    let url = source.stream_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    let cache_key = probe::stream_cache_key(url.as_str());
    let cached = with_db(state.db_path, move |conn| queries::get_probe_cache(conn, &cache_key)).await?;

    Ok(axum::Json(match cached {
        Some(row) => {
            let audio_tracks: Vec<probe::AudioTrack> =
                row.audio_tracks_json.as_deref().and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default();
            let subtitle_tracks: Vec<probe::SubtitleTrack> =
                row.subtitle_tracks_json.as_deref().and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default();
            PlaybackTierResponse {
                tier: Some(row.play_tier as u8),
                video_codec: row.video_codec,
                audio_codec: row.audio_codec,
                container: row.container,
                duration_s: row.duration_s,
                audio_tracks: audio_tracks.into_iter().map(Into::into).collect(),
                subtitle_tracks: subtitle_tracks
                    .into_iter()
                    .filter(|t| t.text)
                    .map(|t| SubtitleTrackSummary { index: t.index, language: t.language, codec_name: t.codec_name })
                    .collect(),
            }
        }
        None => PlaybackTierResponse {
            tier: None,
            video_codec: None,
            audio_codec: None,
            container: None,
            duration_s: None,
            audio_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
        },
    }))
}

/// Mensajes en español, pensados para enseñarse tal cual al usuario (F9: "mensajes que
/// digan qué hacer"), no solo para depurar. Sin inventar causas que no se puedan
/// distinguir de verdad: cuando la causa real es ambigua (p. ej. "no se pudo conectar"
/// puede ser límite de conexiones, cuenta caída o sin red), el mensaje lista las
/// candidatas en vez de fingir certeza que no se tiene.
#[derive(Debug, thiserror::Error)]
enum StreamError {
    #[error("No se encontró este contenido en el catálogo.")]
    NotFound,
    #[error("kind inválido: debe ser channel, movie o episode")]
    BadKind,
    #[error("El proveedor de este contenido ya no está configurado.")]
    ProviderNotFound,
    #[error("No se pudo preparar la conexión con el proveedor: {0}")]
    Source(String),
    #[error("La cuenta de este proveedor caducó. Revisa tu suscripción en Ajustes.")]
    AccountExpired,
    #[error("No se pudo conectar con el proveedor. Puede que hayas alcanzado el límite de conexiones simultáneas, que la cuenta esté caída, o que no haya conexión a internet. Inténtalo de nuevo en un momento.")]
    Unreachable,
    #[error("{}", no_data_message(*max_connections))]
    NoData { max_connections: i64 },
    #[error("ffmpeg o ffprobe no están instalados todavía (se empaquetan como sidecar en una fase posterior).")]
    BinariesMissing,
    #[error("Este contenido necesita mpv como reproductor externo y no está instalado. Instálalo y vuelve a intentarlo, o usa \"Abrir con mpv\" una vez lo tengas.")]
    MpvMissing,
    #[error("Este contenido no se puede reproducir en la ventana de la app (códec no compatible). Usa \"Abrir con mpv\" para reproducirlo en una ventana externa.")]
    NeedsMpv,
    #[error("No se pudo abrir mpv: {0}")]
    MpvLaunchFailed(String),
    #[error("Este canal no ofrece contenido pasado (catch-up) según el proveedor.")]
    CatchupNotSupported,
    #[error("Ese momento está fuera del rango de contenido pasado que guarda el proveedor para este canal.")]
    CatchupOutOfRange,
    #[error("error de base de datos: {0}")]
    Db(#[from] DbError),
    #[error("error interno")]
    Internal,
}

impl StreamError {
    fn status(&self) -> StatusCode {
        match self {
            StreamError::NotFound | StreamError::ProviderNotFound => StatusCode::NOT_FOUND,
            StreamError::BadKind => StatusCode::BAD_REQUEST,
            StreamError::Source(_) | StreamError::Unreachable | StreamError::MpvLaunchFailed(_) => StatusCode::BAD_GATEWAY,
            StreamError::NoData { .. } => StatusCode::GATEWAY_TIMEOUT,
            StreamError::AccountExpired => StatusCode::PAYMENT_REQUIRED,
            StreamError::BinariesMissing | StreamError::MpvMissing => StatusCode::SERVICE_UNAVAILABLE,
            StreamError::NeedsMpv => StatusCode::NOT_IMPLEMENTED,
            StreamError::CatchupNotSupported => StatusCode::NOT_FOUND,
            StreamError::CatchupOutOfRange => StatusCode::BAD_REQUEST,
            StreamError::Db(_) | StreamError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Lo más probable cuando el proveedor acepta la conexión y no manda nada es que la cuenta
/// ya esté en uso en otro sitio (medido con la cuenta real del usuario, de 1 conexión).
fn no_data_message(max_connections: i64) -> String {
    let limit = if max_connections == 1 { "1 conexión".to_string() } else { format!("{max_connections} conexiones") };
    format!(
        "El proveedor no ha enviado vídeo en {} segundos. Si tienes esta cuenta abierta en otro dispositivo (tele, móvil, otra app), ciérralo: solo permite {limit} a la vez. Si no, puede que el canal esté caído.",
        FIRST_DATA_TIMEOUT.as_secs()
    )
}

/// Cuánto se espera el primer trozo de vídeo de un remux antes de darlo por perdido. Lo
/// normal con un proveedor real es 2,5–5 s; sin este límite, el reproductor se quedaba
/// cargando para siempre sin decir por qué.
const FIRST_DATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(if cfg!(test) { 2 } else { 20 });

impl IntoResponse for StreamError {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

/// El motivo del último fallo al abrir este stream (`kind` = `channel`, `movie`,
/// `episode` o `catchup`), o 204 si el último intento abrió bien. No toca al proveedor.
async fn get_last_error(State(state): State<StreamState>, AxumPath((kind, id)): AxumPath<(String, i64)>) -> Response {
    let failure = state.last_errors.lock().ok().and_then(|map| map.get(&format!("{kind}:{id}")).cloned());
    match failure {
        Some(failure) => axum::Json(failure).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

#[derive(Deserialize)]
struct SubtitleCuesQuery {
    track: i64,
    /// El `?start=` del stream al que pertenecen (0 o ausente si arrancó del principio).
    start: Option<f64>,
    /// Cuántas frases tiene ya el cliente: solo se devuelven las siguientes.
    from: Option<usize>,
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SubtitleCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SubtitleCuesResponse {
    /// `false` si el remux en curso no es de este ítem con esta pista (p. ej. ya se
    /// cambió de contenido): el cliente deja de preguntar.
    pub active: bool,
    pub cues: Vec<SubtitleCue>,
}

/// Frases de subtítulos que el remux en curso lleva escritas (F12). Lee el fichero
/// temporal; no toca al proveedor.
async fn get_subtitle_cues(
    State(state): State<StreamState>,
    AxumPath((kind, id)): AxumPath<(String, i64)>,
    Query(query): Query<SubtitleCuesQuery>,
) -> Result<axum::Json<SubtitleCuesResponse>, StreamError> {
    let key = subtitle_session_key(&kind, id, query.track, query.start.filter(|s| *s > 0.0));
    let path = state.subtitles.lock().ok().and_then(|current| current.as_ref().filter(|s| s.key == key).map(|s| s.path.clone()));
    let Some(path) = path else {
        return Ok(axum::Json(SubtitleCuesResponse { active: false, cues: Vec::new() }));
    };
    let cues = tokio::task::spawn_blocking(move || subtitles::read_cues(&path)).await.map_err(|_| StreamError::Internal)?;
    let cues = cues
        .into_iter()
        .skip(query.from.unwrap_or(0))
        .map(|cue| SubtitleCue { start: cue.start, end: cue.end, text: cue.text })
        .collect();
    Ok(axum::Json(SubtitleCuesResponse { active: true, cues }))
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
    /// Perfil que pide la reproducción — de quién es el "continuar viendo"/registro de
    /// nivel que deja esta sesión (F "perfiles Netflix").
    profile_id: i64,
    source: Option<i64>,
    start: Option<f64>,
    /// Índice relativo de pista de audio (F "premium" — selector de pista de audio, ver
    /// `media::probe::AudioTrack`). Solo tiene efecto en nivel 2 (remux): en nivel 1 el
    /// proxy pasa el fichero tal cual, sin ninguna decisión de pistas que tomar aquí.
    audio_track: Option<i64>,
    /// Audio que el cliente decodifica por sí mismo, separado por comas ("ac3,eac3"): ese
    /// se copia en el remux en vez de convertirse a AAC.
    audio_passthrough: Option<String>,
    /// Índice relativo de la pista de subtítulos de texto que se quiere ver (F12). Obliga
    /// a remuxear (nivel 2) aunque el fichero se pudiera servir tal cual.
    subtitle_track: Option<i64>,
}

/// Códecs de `audio_passthrough` que se aceptan: solo los que el remux convertiría.
fn client_audio(list: Option<&str>) -> Vec<String> {
    list.unwrap_or_default()
        .split(',')
        .map(|codec| codec.trim().to_ascii_lowercase())
        .filter(|codec| matches!(codec.as_str(), "ac3" | "eac3" | "dts"))
        .collect()
}

async fn get_stream(
    State(state): State<StreamState>,
    AxumPath((kind, id)): AxumPath<(String, i64)>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let range = headers.get(header::RANGE).cloned();
    let log = Arc::clone(&state.last_errors);
    let audio = client_audio(query.audio_passthrough.as_deref());
    let tracks = TrackChoice { audio: query.audio_track, subtitles: query.subtitle_track };
    let result = handle_stream(state, kind.clone(), id, query.profile_id, query.source, query.start, tracks, &audio, range).await;
    remember_outcome(&log, format!("{kind}:{id}"), result.as_ref());
    match result {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(kind = %kind, id, error = %err, "fallo al servir /stream");
            err.into_response()
        }
    }
}

async fn post_launch_mpv(State(state): State<StreamState>, AxumPath((kind, id)): AxumPath<(String, i64)>, Query(query): Query<StreamQuery>) -> Response {
    match handle_mpv_launch(state, kind.clone(), id, query.profile_id, query.source).await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(kind = %kind, id, error = %err, "fallo al lanzar mpv");
            err.into_response()
        }
    }
}

#[derive(Deserialize)]
struct CatchupQuery {
    profile_id: i64,
    /// Inicio del bloque pedido, en epoch (segundos UTC) — no la hora civil que espera
    /// el proveedor: eso lo traduce `XtreamSource::timeshift_date`.
    start: i64,
    /// Minutos a pedir desde `start`. Por defecto 60: suficiente para un episodio típico
    /// sin arriesgarse a pedir un bloque tan largo que el panel lo rechace.
    duration_min: Option<u32>,
    audio_passthrough: Option<String>,
}

async fn get_catchup(
    State(state): State<StreamState>,
    AxumPath(channel_id): AxumPath<i64>,
    Query(query): Query<CatchupQuery>,
    headers: HeaderMap,
) -> Response {
    let range = headers.get(header::RANGE).cloned();
    let log = Arc::clone(&state.last_errors);
    let audio = client_audio(query.audio_passthrough.as_deref());
    let result = handle_catchup(state, channel_id, query.profile_id, query.start, query.duration_min.unwrap_or(60), &audio, range).await;
    remember_outcome(&log, format!("catchup:{channel_id}"), result.as_ref());
    match result {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(channel_id, error = %err, "fallo al servir /stream/catchup");
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
async fn record_tier(db_path: &PathBuf, profile_id: i64, item_kind: &str, item_id: i64, tier: u8) {
    let db_path = db_path.clone();
    let item_kind = item_kind.to_string();
    let outcome = tokio::task::spawn_blocking(move || -> crate::db::Result<()> {
        let conn = schema::open_readonly_at(&db_path)?;
        queries::record_play_tier(&conn, profile_id, &item_kind, item_id, tier as i64)
    })
    .await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!(error = %err, "no se pudo registrar el nivel de reproducción"),
        Err(err) => tracing::warn!(error = %err, "la tarea de registro de nivel de reproducción falló"),
    }
}

/// Pistas elegidas en el reproductor: de audio y de subtítulos, en índice relativo.
#[derive(Clone, Copy, Default)]
struct TrackChoice {
    audio: Option<i64>,
    subtitles: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
async fn handle_stream(
    state: StreamState,
    kind_str: String,
    id: i64,
    profile_id: i64,
    source_id: Option<i64>,
    start_seconds: Option<f64>,
    tracks: TrackChoice,
    client_audio: &[String],
    range_header: Option<HeaderValue>,
) -> Result<Response, StreamError> {
    let media_kind = parse_kind(&kind_str)?;
    let (provider_id, remote_id, container_extension) = resolve_stream_target(&state.db_path, media_kind, id, source_id).await?;

    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;

    // Falla rápido y con un mensaje concreto si la cuenta ya caducó (dato fiable: viene
    // de la última sincronización), en vez de intentar abrir una conexión que el
    // proveedor va a rechazar de todas formas — mensajes claros de F9.
    if let Some(expires_at) = provider_row.expires_at {
        if expires_at < crate::db::now_epoch() {
            return Err(StreamError::AccountExpired);
        }
    }

    let max_connections = provider_row.max_connections;
    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;

    let item = StreamRef { kind: media_kind, remote_id, container_extension };
    let primary_url = source.stream_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    // A partir de aquí se abre (o se reutiliza) LA conexión de stream: cierra la
    // anterior y espera su muerte real antes de devolver el permiso (regla R1).
    let permit = state.stream_guard.acquire().await;
    // El ffmpeg anterior ya está muerto: sus subtítulos dejan de valer.
    replace_subtitle_session(&state.subtitles, None);

    // Nivel 1a: HLS reconocido por la extensión, sin necesidad de sonda. No reescribe
    // las URIs internas del manifiesto (ver docs/DECISIONS.md): no se ha podido probar
    // contra un proveedor que sirva HLS de verdad, ninguno de los dos del usuario lo hace.
    if primary_url.path().to_ascii_lowercase().ends_with(".m3u8") {
        record_tier(&state.db_path, profile_id, &kind_str, id, 1).await;
        return proxy_passthrough(primary_url, range_header, permit, Arc::clone(&state.stream_guard)).await;
    }

    let (ffmpeg_path, ffprobe_path) = match (&state.ffmpeg_path, &state.ffprobe_path) {
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg.clone(), ffprobe.clone()),
        _ => return Err(StreamError::BinariesMissing),
    };

    let (resolved_url, tier, audio_codec) =
        resolve_tier_and_url(&source, &item, primary_url, &ffprobe_path, &state.db_path).await?;
    // Solo una pista que la sonda vio y que es de texto: con un índice que no existe,
    // ffmpeg abortaría el remux entero, vídeo incluido.
    let subtitle_track = text_subtitle_track(&state.db_path, &resolved_url, tracks.subtitles).await;
    // Los subtítulos salen del remux: un MP4 que iría tal cual (nivel 1) pasa por él.
    let tier = if tier == 1 && subtitle_track.is_some() { 2 } else { tier };
    record_tier(&state.db_path, profile_id, &kind_str, id, tier).await;

    match tier {
        1 => proxy_passthrough(resolved_url, range_header, permit, Arc::clone(&state.stream_guard)).await,
        2 => {
            let codec = selected_track_codec(&state.db_path, &resolved_url, tracks.audio).await.or(audio_codec);
            let live = matches!(item.kind, MediaKind::Live);
            let subtitles_path =
                subtitle_track.and_then(|track| replace_subtitle_session(&state.subtitles, Some(subtitle_session_key(&kind_str, id, track, start_seconds.filter(|s| *s > 0.0)))));
            let subtitles = subtitle_track.zip(subtitles_path.as_deref());
            let options = RemuxOptions {
                audio_codec: codec.as_deref(),
                client_audio,
                live,
                start_seconds,
                audio_track: tracks.audio,
                subtitles,
                ..Default::default()
            };
            if live {
                spawn_live_response(&ffmpeg_path, resolved_url.as_str(), options, max_connections, id, permit, &state).await
            } else {
                spawn_remux_response(&ffmpeg_path, resolved_url.as_str(), options, max_connections, permit, Arc::clone(&state.stream_guard)).await
            }
        }
        _ => Err(StreamError::NeedsMpv),
    }
}

/// La pista de subtítulos pedida, si la sonda la conoce y es de texto.
async fn text_subtitle_track(db_path: &Path, url: &Url, requested: Option<i64>) -> Option<i64> {
    let track = requested?;
    let cache_key = probe::stream_cache_key(url.as_str());
    let row = with_db(db_path.to_path_buf(), move |conn| queries::get_probe_cache(conn, &cache_key)).await.ok().flatten()?;
    let tracks: Vec<probe::SubtitleTrack> = serde_json::from_str(row.subtitle_tracks_json.as_deref()?).ok()?;
    tracks.into_iter().any(|t| t.index == track && t.text).then_some(track)
}

/// Nivel 3 (F9): lanza mpv en su propia ventana con la URL real del proveedor (no la de
/// `/stream/*`: mpv existe justo para lo que el proxy de nivel 2 no puede remuxear sin
/// recodificar). Responde en cuanto mpv arranca, no cuando termina — a diferencia de
/// nivel 1/2, aquí no hay cuerpo de respuesta al que atar la duración de la sesión, así
/// que `StreamGuard::run_until_exit_or_replaced` la mantiene ocupada en una tarea aparte
/// hasta que el usuario cierre la ventana de mpv o se abra otra sesión.
///
/// **Sin verificar de verdad** (ver `media::mpv`): no hay mpv instalado en la máquina
/// donde se ha escrito esto.
async fn handle_mpv_launch(state: StreamState, kind_str: String, id: i64, profile_id: i64, source_id: Option<i64>) -> Result<Response, StreamError> {
    let media_kind = parse_kind(&kind_str)?;
    let (provider_id, remote_id, container_extension) = resolve_stream_target(&state.db_path, media_kind, id, source_id).await?;

    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;
    if let Some(expires_at) = provider_row.expires_at {
        if expires_at < crate::db::now_epoch() {
            return Err(StreamError::AccountExpired);
        }
    }

    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;
    let item = StreamRef { kind: media_kind, remote_id, container_extension };
    let url = source.stream_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    let mpv_path = state.mpv_path.clone().ok_or(StreamError::MpvMissing)?;
    let permit = state.stream_guard.acquire().await;

    let ipc_pipe = mpv::new_ipc_pipe_path();
    let child = mpv::spawn_mpv(&mpv_path, url.as_str(), DEFAULT_USER_AGENT, &ipc_pipe).map_err(|err| {
        tracing::error!(error = %err, "no se pudo lanzar mpv");
        StreamError::MpvLaunchFailed(err.to_string())
    })?;
    state.stream_guard.attach_process(&permit, child).await;
    record_tier(&state.db_path, profile_id, &kind_str, id, 3).await;

    let guard = Arc::clone(&state.stream_guard);
    tokio::spawn(async move { guard.run_until_exit_or_replaced(permit).await });

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /stream/catchup/:channel_id` — timeshift (F9): reproduce un bloque pasado de un
/// canal en vez del directo. Solo se ofrece si `channels.has_catchup` y el `start` pedido
/// cae dentro de `channels.catchup_days`; fuera de eso ni se intenta abrir conexión.
async fn handle_catchup(
    state: StreamState,
    channel_id: i64,
    profile_id: i64,
    start_epoch: i64,
    duration_min: u32,
    client_audio: &[String],
    range_header: Option<HeaderValue>,
) -> Result<Response, StreamError> {
    let channel = with_db(state.db_path.clone(), move |conn| reads::resolve_channel_catchup(conn, channel_id))
        .await?
        .ok_or(StreamError::NotFound)?;
    if !channel.has_catchup {
        return Err(StreamError::CatchupNotSupported);
    }

    let now = crate::db::now_epoch();
    let oldest_allowed = now - (channel.catchup_days as i64) * 86_400;
    let end_epoch = start_epoch + (duration_min as i64) * 60;
    if start_epoch < oldest_allowed || start_epoch > now || end_epoch > now {
        return Err(StreamError::CatchupOutOfRange);
    }

    let provider_row = with_db(state.db_path.clone(), move |conn| queries::get_provider(conn, channel.provider_id))
        .await?
        .ok_or(StreamError::ProviderNotFound)?;
    if let Some(expires_at) = provider_row.expires_at {
        if expires_at < now {
            return Err(StreamError::AccountExpired);
        }
    }

    let max_connections = provider_row.max_connections;
    let source = build_source_for_provider(provider_row).await.map_err(StreamError::Source)?;
    let item = CatchupRef { remote_id: channel.remote_id, start_epoch, duration_min };
    let primary_url = source.catchup_url(&item).map_err(|err| StreamError::Source(err.to_string()))?;

    let permit = state.stream_guard.acquire().await;
    replace_subtitle_session(&state.subtitles, None);

    let (ffmpeg_path, ffprobe_path) = match (&state.ffmpeg_path, &state.ffprobe_path) {
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg.clone(), ffprobe.clone()),
        _ => return Err(StreamError::BinariesMissing),
    };

    let (resolved_url, tier, audio_codec) =
        resolve_catchup_tier_and_url(&source, &item, primary_url, &ffprobe_path, &state.db_path).await?;
    record_tier(&state.db_path, profile_id, "channel", channel_id, tier).await;

    match tier {
        1 => proxy_passthrough(resolved_url, range_header, permit, Arc::clone(&state.stream_guard)).await,
        2 => {
            // Un programa ya emitido tiene principio y fin: como una película, sin `nobuffer`.
            let options = RemuxOptions { audio_codec: audio_codec.as_deref(), client_audio, ..Default::default() };
            spawn_remux_response(&ffmpeg_path, resolved_url.as_str(), options, max_connections, permit, Arc::clone(&state.stream_guard)).await
        }
        _ => Err(StreamError::NeedsMpv),
    }
}

/// Igual que `resolve_tier_and_url`, pero para catch-up: la sonda y la caché son
/// idénticas, solo cambia de dónde sale la URL principal y la alternativa
/// (`Source::catchup_url`/`catchup_url_fallback` en vez de `stream_url`/`stream_url_fallback`).
async fn resolve_catchup_tier_and_url(
    source: &Arc<dyn Source + Send + Sync>,
    item: &CatchupRef,
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
            let fallback = source.catchup_url_fallback(item).map_err(|err| StreamError::Source(err.to_string()))?;
            let Some(fallback_url) = fallback else {
                tracing::warn!(error = %primary_err, "no se pudo inspeccionar el catch-up y no hay URL alternativa");
                return Err(StreamError::Unreachable);
            };
            match probe::probe_url(ffprobe_path, fallback_url.as_str(), DEFAULT_USER_AGENT).await {
                Ok(result) => (fallback_url, result),
                Err(fallback_err) => {
                    tracing::warn!(primary = %primary_err, fallback = %fallback_err, "ninguna de las dos formas de la URL de catch-up respondió");
                    return Err(StreamError::Unreachable);
                }
            }
        }
    };

    let tier = probe::decide_tier(&probe_result.container, probe_result.video_codec.as_deref(), probe_result.audio_codec.as_deref());
    let store_key = probe::stream_cache_key(resolved_url.as_str());
    let video = probe_result.video_codec.clone();
    let audio = probe_result.audio_codec.clone();
    let container = probe_result.container.clone();
    let audio_tracks_json = serde_json::to_string(&probe_result.audio_tracks).ok();
    let subtitle_tracks_json = serde_json::to_string(&probe_result.subtitle_tracks).ok();
    // Catch-up es contenido con fin (el bloque pedido), pero su "duración" no es la del
    // programa: no tiene sentido pintarla como barra de búsqueda editorial, así que no
    // se guarda (igual que el directo en `resolve_tier_and_url`).
    with_db(db_path.clone(), move |conn| {
        queries::set_probe_cache(
            conn,
            &store_key,
            video.as_deref(),
            audio.as_deref(),
            Some(&container),
            tier as i64,
            None,
            audio_tracks_json.as_deref(),
            subtitle_tracks_json.as_deref(),
        )
    })
    .await?;

    Ok((resolved_url, tier, probe_result.audio_codec))
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
    let audio_tracks_json = serde_json::to_string(&probe_result.audio_tracks).ok();
    let subtitle_tracks_json = serde_json::to_string(&probe_result.subtitle_tracks).ok();
    with_db(db_path.clone(), move |conn| {
        queries::set_probe_cache(
            conn,
            &store_key,
            video.as_deref(),
            audio.as_deref(),
            Some(&container),
            tier as i64,
            duration_s,
            audio_tracks_json.as_deref(),
            subtitle_tracks_json.as_deref(),
        )
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

/// Códec de la pista de audio elegida en el selector, si se eligió una y la sonda la
/// conoce. Puede no coincidir con el de la primera pista (`probe_cache.audio_codec`),
/// y es el que decide cómo tratar el audio en el remux.
async fn selected_track_codec(db_path: &Path, url: &Url, audio_track: Option<i64>) -> Option<String> {
    let track = audio_track?;
    let cache_key = probe::stream_cache_key(url.as_str());
    let row = with_db(db_path.to_path_buf(), move |conn| queries::get_probe_cache(conn, &cache_key)).await.ok().flatten()?;
    let tracks: Vec<probe::AudioTrack> = serde_json::from_str(row.audio_tracks_json.as_deref()?).ok()?;
    tracks.into_iter().find(|t| t.index == track)?.codec_name
}

/// Nivel 2: remux a fMP4 con ffmpeg, un proceso vivo por sesión (ver
/// `util::stream_guard`). `start_seconds` implementa la búsqueda en VOD: el cliente pide
/// un stream nuevo con `?start=<segundos>` y aquí se traduce a un `-ss` de entrada.
/// `max_connections` solo sirve para explicar un remux que no recibe nada.
async fn spawn_remux_response(
    ffmpeg_path: &Path,
    url: &str,
    options: RemuxOptions<'_>,
    max_connections: i64,
    permit: StreamPermit,
    guard: Arc<StreamGuard>,
) -> Result<Response, StreamError> {
    let generation = permit.generation();
    let (first, reader) = start_remux(ffmpeg_path, url, &options, max_connections, &permit, &guard).await?;
    let rest = reader.map(|item| item.map_err(|err| Box::new(err) as BoxError));
    let inner: BoxedByteStream = Box::pin(futures_util::stream::once(async move { Ok::<Bytes, BoxError>(first) }).chain(rest));
    let body = Body::from_stream(PermitBoundBody { inner, permit: Some(permit), guard, generation });
    mp4_response(body)
}

/// Lanza ffmpeg y espera su primer trozo antes de responder: si el proveedor acepta la
/// conexión y no manda nada (cuenta en uso en otro sitio, canal caído), el cliente recibe
/// un error que explica qué pasa, en vez de un vídeo que carga para siempre.
async fn start_remux(
    ffmpeg_path: &Path,
    url: &str,
    options: &RemuxOptions<'_>,
    max_connections: i64,
    permit: &StreamPermit,
    guard: &Arc<StreamGuard>,
) -> Result<(Bytes, ReaderStream<tokio::process::ChildStdout>), StreamError> {
    let mut child = ffmpeg::spawn_remux(ffmpeg_path, url, DEFAULT_USER_AGENT, options).map_err(|err| {
        tracing::error!(error = %err, "no se pudo lanzar ffmpeg para el remux");
        StreamError::BinariesMissing
    })?;
    let stdout = child.stdout.take().ok_or(StreamError::Internal)?;
    guard.attach_process(permit, child).await;
    let generation = permit.generation();

    let mut reader = ReaderStream::new(stdout);
    match tokio::time::timeout(FIRST_DATA_TIMEOUT, reader.next()).await {
        Ok(Some(Ok(bytes))) => Ok((bytes, reader)),
        Ok(_) => {
            // ffmpeg terminó sin sacar nada: el proveedor rechazó o cortó la conexión.
            guard.release_process(generation).await;
            Err(StreamError::Unreachable)
        }
        Err(_) => {
            tracing::warn!(seconds = FIRST_DATA_TIMEOUT.as_secs(), "el remux no recibió nada del proveedor");
            guard.release_process(generation).await;
            Err(StreamError::NoData { max_connections })
        }
    }
}

fn mp4_response(body: Body) -> Result<Response, StreamError> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .body(body)
        .map_err(|_| StreamError::Internal)
}

/// Trozos de la salida en directo que pueden esperar a que el reproductor los lea. Lleno,
/// es que el reproductor dejó de leer (pausa): se deja de mandarle el directo.
const LIVE_BUFFER_CHUNKS: usize = 256;

/// Directo con grabación para pausar y rebobinar (F13). El mismo ffmpeg saca el directo
/// por la respuesta y graba en disco (ver `server::timeshift`). A diferencia del VOD, la
/// sesión no muere con la respuesta: el reproductor en pausa deja de leer el directo pero
/// sigue usando la grabación. ffmpeg se lee siempre, lea o no el reproductor: si dejara
/// de leerse, se pararía también la grabación y la conexión con el proveedor.
async fn spawn_live_response(
    ffmpeg_path: &Path,
    url: &str,
    mut options: RemuxOptions<'_>,
    max_connections: i64,
    channel_id: i64,
    permit: StreamPermit,
    state: &StreamState,
) -> Result<Response, StreamError> {
    let dir = match timeshift::new_session_dir() {
        Ok(dir) => Some(dir),
        Err(err) => {
            tracing::warn!(error = %err, "no se pudo preparar la grabación del directo; se ve sin poder pausar");
            None
        }
    };
    options.timeshift = dir.as_deref();
    options.timeshift_window_seconds = timeshift::WINDOW_SECONDS;

    let guard = Arc::clone(&state.stream_guard);
    let (first, mut reader) = match start_remux(ffmpeg_path, url, &options, max_connections, &permit, &guard).await {
        Ok(started) => started,
        Err(err) => {
            if let Some(dir) = &dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            return Err(err);
        }
    };

    let session = Arc::new(TimeshiftSession::new(channel_id, dir, permit.generation()));
    timeshift::register(&state.timeshift, Arc::clone(&session));

    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(LIVE_BUFFER_CHUNKS);
    let draining = Arc::clone(&session);
    tokio::spawn(async move {
        let mut live = Some(tx);
        let mut pending = Some(first);
        loop {
            let chunk = match pending.take() {
                Some(chunk) => chunk,
                None => match reader.next().await {
                    Some(Ok(chunk)) => chunk,
                    _ => break,
                },
            };
            if let Some(tx) = &live {
                // Lleno (pausa) o cerrado (el reproductor pasó a la grabación o se fue): a
                // partir de aquí se descarta, la grabación sigue.
                if tx.try_send(chunk).is_err() {
                    live = None;
                    draining.set_reading_live(false);
                }
            }
        }
        draining.set_reading_live(false);
    });

    let slot = Arc::clone(&state.timeshift);
    let watched = Arc::clone(&session);
    tokio::spawn(async move {
        let wanted = Arc::clone(&watched);
        guard.run_while(permit, move || wanted.is_wanted()).await;
        timeshift::finish(&slot, &watched);
    });

    let body = Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (Ok::<Bytes, BoxError>(chunk), rx))
    }));
    mp4_response(body)
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
    /// Borra la credencial de prueba al terminar y, mientras vive, tiene el cerrojo del
    /// keychain (`keychain::TEST_LOCK`): el núcleo vuelve a leer la credencial al abrir el
    /// stream, y con otros tests escribiendo a la vez el almacén de Windows devolvía
    /// `NoEntry` de forma intermitente.
    struct KeychainGuard(String, #[allow(dead_code)] tokio::sync::MutexGuard<'static, ()>);
    impl Drop for KeychainGuard {
        fn drop(&mut self) {
            let _ = keychain::delete_secret(&self.0);
        }
    }

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vibe-iptv-stream-test-{label}-{}-{}.sqlite3", std::process::id(), rand_suffix()))
    }

    /// Aleatorio de verdad, no la hora: dos tests en paralelo podían sacar el mismo
    /// nanosegundo, abrir el mismo fichero SQLite y fallar de forma intermitente.
    fn rand_suffix() -> u64 {
        use rand::RngExt;
        rand::rng().random()
    }

    async fn generate_sample(ffmpeg_path: &Path, dir: &Path, filename: &str, container: &str) -> PathBuf {
        let out = dir.join(filename);
        let status = tokio::process::Command::new(ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=6:size=64x64:rate=10", "-f", "lavfi", "-i", "sine=duration=6"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-t", "6", "-f", container])
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success(), "no se pudo generar el fichero de prueba {filename}");
        out
    }

    /// Igual que `generate_sample`, pero con dos pistas de audio distinguibles por su
    /// número de canales (mono la primera, estéreo la segunda) — para probar de verdad
    /// que `?audio_track=N` selecciona la pista correcta en el remux, no solo que el
    /// parámetro se acepta sin error.
    async fn generate_sample_with_two_audio_tracks(ffmpeg_path: &Path, dir: &Path, filename: &str) -> PathBuf {
        let out = dir.join(filename);
        let status = tokio::process::Command::new(ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=64x64:rate=10"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=2"])
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-ac:a:1", "2", "-t", "2", "-f", "mpegts"])
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success(), "no se pudo generar el fichero de prueba con dos pistas de audio {filename}");
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

    async fn seed_provider_with_credential(db_path: &Path, base_url: &str, credential_suffix: &str) -> (i64, KeychainGuard) {
        let lock = keychain::TEST_LOCK.lock().await;
        let conn = open_connection_at(db_path).unwrap();
        let provider_id =
            write_queries::insert_provider(&conn, ProviderKind::Xtream, "P", base_url, Some("user"), 1, None).unwrap();
        let credential_ref = format!("vibe-iptv-stream-test-{}-{credential_suffix}", std::process::id());
        keychain::store_secret(&credential_ref, "pass").unwrap();
        write_queries::set_provider_credential_ref(&conn, provider_id, &credential_ref).unwrap();
        (provider_id, KeychainGuard(credential_ref, lock))
    }

    fn base_state(db_path: PathBuf, ffmpeg_path: Option<PathBuf>, ffprobe_path: Option<PathBuf>) -> StreamState {
        StreamState {
            db_path,
            stream_guard: StreamGuard::new(),
            ffmpeg_path,
            ffprobe_path,
            mpv_path: None,
            last_errors: Default::default(),
            subtitles: Default::default(),
            timeshift: Default::default(),
        }
    }

    #[tokio::test]
    async fn returns_400_for_an_invalid_kind() {
        let db_path = temp_db_path("badkind");
        open_connection_at(&db_path).unwrap();
        let state = base_state(db_path.clone(), None, None);

        let response = stream_router(state)
            .oneshot(Request::builder().uri("/stream/podcast/1?profile_id=1").body(Body::empty()).unwrap())
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
            .oneshot(Request::builder().uri("/stream/channel/9999?profile_id=1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(&db_path);
    }

    /// El reproductor pregunta aquí por qué falló un stream, en vez de volver a pedirlo
    /// (que abría otra sesión con el proveedor y mataba la que se estaba viendo).
    #[tokio::test]
    async fn last_error_reports_why_a_stream_failed_without_opening_it_again() {
        let db_path = temp_db_path("lasterror");
        open_connection_at(&db_path).unwrap();
        let router = stream_router(base_state(db_path.clone(), None, None));

        let before = router.clone().oneshot(Request::builder().uri("/api/playback/last-error/channel/9999").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(before.status(), StatusCode::NO_CONTENT);

        let failed = router.clone().oneshot(Request::builder().uri("/stream/channel/9999?profile_id=1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(failed.status(), StatusCode::NOT_FOUND);

        let after = router.oneshot(Request::builder().uri("/api/playback/last-error/channel/9999").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(after.status(), StatusCode::OK);
        let body = after.into_body().collect().await.unwrap().to_bytes();
        let failure: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(failure["status"], 404);
        assert!(!failure["message"].as_str().unwrap().is_empty());
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
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "mp4").await;
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
            .oneshot(Request::builder().uri(format!("/stream/movie/{movie_id}?profile_id=1")).body(Body::empty()).unwrap())
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
        assert!(duration.is_some_and(|d| d > 5.5 && d < 7.0), "duración inesperada: {duration:?}");

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
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "ts").await;
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

        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path.clone()));
        let response = stream_router(state)
            .oneshot(Request::builder().uri(format!("/stream/channel/{channel_id}?profile_id=1")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::CONTENT_TYPE).unwrap(), "video/mp4");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.is_empty());
        assert_eq!(&body[4..8], b"ftyp");
        // La cabecera sola no prueba nada (ver `media::count_packets`).
        let remuxed = dir.join("remuxed-live.mp4");
        std::fs::write(&remuxed, &body).unwrap();
        assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "v") > 0, "el remux del directo no lleva vídeo");
        assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "a") > 0, "el remux del directo no lleva audio");

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

    /// Prueba de verdad del selector de pista de audio (F "premium", pendiente desde el
    /// plan original de F6) — no solo que `?audio_track=N` se acepta sin error, sino que
    /// el remux resultante lleva de verdad la pista pedida y ninguna otra. Genera un
    /// stream con dos pistas distinguibles por su número de canales (mono/estéreo, ver
    /// `generate_sample_with_two_audio_tracks`) y confirma con un segundo ffprobe sobre
    /// la respuesta cuál de las dos quedó dentro.
    #[tokio::test]
    async fn selecting_an_audio_track_maps_that_exact_track_into_the_remux() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-audiotrack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = generate_sample_with_two_audio_tracks(&ffmpeg_path, &dir, "sample.ts").await;
        let base_url = spawn_file_server("/live/user/pass/1.ts", sample).await;

        let db_path = temp_db_path("audiotrack");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "audiotrack").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", false, 0);

        // Pista 1 (índice relativo) es la estéreo según `generate_sample_with_two_audio_tracks`.
        let state = base_state(db_path.clone(), Some(ffmpeg_path.clone()), Some(ffprobe_path.clone()));
        let response = stream_router(state)
            .oneshot(Request::builder().uri(format!("/stream/channel/{channel_id}?profile_id=1&audio_track=1")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.is_empty());

        let remuxed_path = dir.join("remuxed-track1.mp4");
        std::fs::write(&remuxed_path, &body).unwrap();
        let probed = probe::probe_url(&ffprobe_path, &remuxed_path.to_string_lossy(), "vibe-iptv-test").await.unwrap();
        assert_eq!(probed.audio_tracks.len(), 1, "el remux con una pista elegida debe llevar solo esa, no las dos originales");
        assert_eq!(probed.audio_tracks[0].channels, Some(2), "se pidió audio_track=1 (la estéreo) y no llegó");
        assert!(crate::media::count_packets(&ffprobe_path, &remuxed_path, "a") > 0, "la pista elegida está declarada pero vacía");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&db_path);
    }

    /// Subtítulos de punta a punta (F12): un MKV con dos pistas SRT; se pide la segunda y
    /// sus frases llegan por la API con sus tiempos, mientras el vídeo sigue saliendo por
    /// el mismo remux (una sola conexión con el proveedor, regla R1).
    #[tokio::test]
    async fn a_text_subtitle_track_comes_out_of_the_same_remux_as_cues() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-subs-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("es.srt"), "1\n00:00:01,000 --> 00:00:02,500\nHola mundo\n\n2\n00:00:03,000 --> 00:00:04,000\nAdiós\n\n").unwrap();
        std::fs::write(dir.join("en.srt"), "1\n00:00:01,000 --> 00:00:02,500\nHello world\n\n2\n00:00:03,000 --> 00:00:04,000\nBye\n\n").unwrap();
        let sample = dir.join("sample.mkv");
        let status = tokio::process::Command::new(&ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=6:size=64x64:rate=10", "-f", "lavfi", "-i", "sine=duration=6"])
            .arg("-i")
            .arg(dir.join("es.srt"))
            .arg("-i")
            .arg(dir.join("en.srt"))
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:s", "-map", "3:s", "-c:v", "libx264", "-c:a", "aac", "-c:s", "srt"])
            .args(["-metadata:s:s:0", "language=spa", "-metadata:s:s:1", "language=eng", "-t", "6", "-f", "matroska"])
            .arg(&sample)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success(), "no se pudo generar el MKV con subtítulos");
        let base_url = spawn_file_server("/movie/user/pass/1.mkv", sample).await;

        let db_path = temp_db_path("subs");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "subs").await;
        let movie_id: i64 = {
            let conn = open_connection_at(&db_path).unwrap();
            let raw = RawMovie {
                remote_id: "1".into(),
                category_remote_id: None,
                raw_title: "Peli".into(),
                container_extension: Some("mkv".into()),
                logo_url: None,
                added_at: None,
                rating: None,
                tmdb_id: None,
            };
            let fields = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-2020", quality: None, language: None };
            write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
            conn.query_row("SELECT id FROM movies WHERE match_key = 'peli-2020'", [], |r| r.get(0)).unwrap()
        };

        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path.clone()));
        let response = get_oneshot(state.clone(), &format!("/stream/movie/{movie_id}?profile_id=1&subtitle_track=1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let remuxed = dir.join("remuxed-subs.mp4");
        std::fs::write(&remuxed, &body).unwrap();
        assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "v") > 0, "con subtítulos el remux no lleva vídeo");
        assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "a") > 0, "con subtítulos el remux no lleva audio");
        assert_eq!(crate::media::count_packets(&ffprobe_path, &remuxed, "s"), 0, "los subtítulos no van dentro del fMP4");

        // El fichero lo escribe otro hilo de ffmpeg: se le da un momento por si acaso.
        let mut cues = serde_json::Value::Null;
        for _ in 0..30 {
            let response = get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=1")).await;
            cues = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
            if cues["cues"].as_array().is_some_and(|c| c.len() == 2) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(cues["active"], true);
        let list = cues["cues"].as_array().unwrap();
        assert_eq!(list.len(), 2, "frases inesperadas: {cues}");
        assert_eq!(list[0]["text"], "Hello world", "se pidió la pista 1 (inglés)");
        assert!((list[0]["start"].as_f64().unwrap() - 1.0).abs() < 0.01);
        assert!((list[0]["end"].as_f64().unwrap() - 2.5).abs() < 0.01);

        let from_one: serde_json::Value = serde_json::from_slice(
            &get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=1&from=1")).await.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(from_one["cues"].as_array().unwrap().len(), 1);
        assert_eq!(from_one["cues"][0]["text"], "Bye");

        let other_track: serde_json::Value = serde_json::from_slice(
            &get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=0")).await.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(other_track["active"], false, "la pista 0 no es la de la sesión en curso");

        // Tras una búsqueda (`?start=2`) el cero de los subtítulos es el punto pedido,
        // igual que el del vídeo: "Bye" (3 s en el original) cae en el segundo 1. La
        // frase que estaba en pantalla en el segundo 2 se pierde.
        let response = get_oneshot(state.clone(), &format!("/stream/movie/{movie_id}?profile_id=1&start=2&subtitle_track=1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.into_body().collect().await.unwrap().to_bytes();
        let mut seeked = serde_json::Value::Null;
        for _ in 0..30 {
            let response = get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=1&start=2")).await;
            seeked = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
            if seeked["cues"].as_array().is_some_and(|c| !c.is_empty()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(seeked["cues"][0]["text"], "Bye", "frases tras buscar: {seeked}");
        assert!((seeked["cues"][0]["start"].as_f64().unwrap() - 1.0).abs() < 0.05, "subtítulos desplazados tras buscar: {seeked}");
        let before_seek: serde_json::Value = serde_json::from_slice(
            &get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=1")).await.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(before_seek["active"], false, "las frases de antes de buscar (otra escala de tiempos) ya no se sirven");

        let tier: serde_json::Value = serde_json::from_slice(
            &get_oneshot(state.clone(), &format!("/api/playback/tier/movie/{movie_id}")).await.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        let tracks = tier["subtitle_tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[1]["language"], "eng");

        // Una pista que la sonda no vio se ignora: el vídeo sale igual, sin subtítulos.
        let response = get_oneshot(state.clone(), &format!("/stream/movie/{movie_id}?profile_id=1&subtitle_track=7")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[4..8], b"ftyp");
        let after: serde_json::Value = serde_json::from_slice(
            &get_oneshot(state.clone(), &format!("/api/playback/subtitles/movie/{movie_id}?track=1")).await.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(after["active"], false, "abrir otro stream cierra la sesión de subtítulos anterior");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&db_path);
    }

    /// Lo que pasó con la cuenta real del usuario (1 conexión, en uso en otro sitio): el
    /// proveedor acepta la conexión y no manda nada. Antes el reproductor se quedaba
    /// cargando para siempre; ahora el núcleo corta y deja el motivo.
    #[tokio::test]
    async fn a_provider_that_accepts_but_never_sends_ends_with_an_explained_error() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\n\r\n").await;
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                });
            }
        });

        let db_path = temp_db_path("nodata");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "nodata").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", false, 0);
        {
            // Ya sondeado antes: se va directo al remux, como un canal que ya se vio.
            let conn = open_connection_at(&db_path).unwrap();
            let key = probe::stream_cache_key(&format!("{base_url}/live/user/pass/1.ts"));
            write_queries::set_probe_cache(&conn, &key, Some("h264"), Some("aac"), Some("mpegts"), 2, None, None, None).unwrap();
        }

        let state = base_state(db_path.clone(), Some(ffmpeg_path), which_on_path("ffprobe"));
        let started = std::time::Instant::now();
        let response = get_oneshot(state.clone(), &format!("/stream/channel/{channel_id}?profile_id=1")).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "tiene que cortar, no esperar indefinidamente");

        let failure = get_oneshot(state, &format!("/api/playback/last-error/channel/{channel_id}")).await;
        let body = failure.into_body().collect().await.unwrap().to_bytes();
        let failure: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(failure["status"], 504);
        assert!(failure["message"].as_str().unwrap().contains("otro dispositivo"), "{failure}");
        let _ = std::fs::remove_file(&db_path);
    }

    /// Sirve `file` a ritmo de directo (trozos con pausas), como un canal de verdad: así
    /// la sesión sigue viva el rato que la prueba necesita para mirar la grabación.
    async fn spawn_paced_file_server(route_path: &str, file: PathBuf) -> String {
        let bytes = Arc::new(std::fs::read(file).unwrap());
        let router = Router::new().route(
            route_path,
            get(move || {
                let bytes = Arc::clone(&bytes);
                async move {
                    let chunks = futures_util::stream::unfold(0usize, move |offset| {
                        let bytes = Arc::clone(&bytes);
                        async move {
                            if offset >= bytes.len() {
                                return None;
                            }
                            if offset > 0 {
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            let end = (offset + 24 * 1024).min(bytes.len());
                            Some((Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&bytes[offset..end])), end))
                        }
                    });
                    ([(header::CONTENT_TYPE, "video/mp2t")], Body::from_stream(chunks))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    /// Pausar el directo (F13), de punta a punta: el mismo remux graba trozos HLS que se
    /// pueden pedir mientras se ve, con el audio MP2 convertido a AAC solo en la grabación
    /// (MSE no lo acepta), la sesión sobrevive a que el reproductor deje de leer el
    /// directo si da señales de vida, y al cerrarla se libera todo: proceso, permiso y
    /// carpeta.
    #[tokio::test]
    async fn live_channels_are_recorded_for_pausing_and_cleaned_up_after() {
        let Some(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };
        let Some(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-stream-timeshift-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = dir.join("canal.ts");
        let status = tokio::process::Command::new(&ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=20", "-f", "lavfi", "-i", "sine=duration=20"])
            .args(["-ac", "2", "-c:v", "libx264", "-preset", "ultrafast", "-g", "25", "-c:a", "mp2", "-b:a", "192k", "-f", "mpegts"])
            .arg(&sample)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let base_url = spawn_paced_file_server("/live/user/pass/1.ts", sample).await;

        let db_path = temp_db_path("timeshift");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "timeshift").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", false, 0);
        let state = base_state(db_path.clone(), Some(ffmpeg_path), Some(ffprobe_path.clone()));

        let response = get_oneshot(state.clone(), &format!("/stream/channel/{channel_id}?profile_id=1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut live = response.into_body().into_data_stream();
        assert!(live.next().await.is_some(), "el directo empieza a llegar");

        // La grabación aparece mientras se ve, con su lista y sus trozos.
        let mut playlist = String::new();
        for _ in 0..80 {
            let response = get_oneshot(state.clone(), &format!("/stream/timeshift/channel/{channel_id}/live.m3u8")).await;
            if response.status() == StatusCode::OK {
                playlist = String::from_utf8(response.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
                if playlist.matches(".m4s").count() >= 2 {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(playlist.contains("#EXT-X-MAP:URI=\"init.mp4\""), "lista: {playlist}");
        let first_segment = playlist.lines().find(|l| l.ends_with(".m4s")).unwrap().to_string();
        let mut recorded = Vec::new();
        for file in ["init.mp4", first_segment.as_str()] {
            let response = get_oneshot(state.clone(), &format!("/stream/timeshift/channel/{channel_id}/{file}")).await;
            assert_eq!(response.status(), StatusCode::OK, "{file}");
            recorded.extend_from_slice(&response.into_body().collect().await.unwrap().to_bytes());
        }
        let recorded_path = dir.join("grabado.mp4");
        std::fs::write(&recorded_path, &recorded).unwrap();
        let probed = probe::probe_url(&ffprobe_path, &recorded_path.to_string_lossy(), "vibe-iptv-test").await.unwrap();
        assert_eq!(probed.video_codec.as_deref(), Some("h264"));
        assert_eq!(probed.audio_codec.as_deref(), Some("aac"), "MP2 se convierte a AAC en la grabación");
        assert!(crate::media::count_packets(&ffprobe_path, &recorded_path, "v") > 0);

        // Un canal que no es el de la grabación no ve nada; un nombre raro, tampoco.
        assert_eq!(get_oneshot(state.clone(), &format!("/stream/timeshift/channel/{}/live.m3u8", channel_id + 1)).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get_oneshot(state.clone(), &format!("/stream/timeshift/channel/{channel_id}/..%2Fsecreto.m4s")).await.status(), StatusCode::BAD_REQUEST);

        // El reproductor deja de leer el directo (pasa a la grabación): con señales de
        // vida, la sesión sigue.
        drop(live);
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let keepalive = Request::builder().method("POST").uri(format!("/api/timeshift/{channel_id}/keepalive")).body(Body::empty()).unwrap();
        assert_eq!(stream_router(state.clone()).oneshot(keepalive).await.unwrap().status(), StatusCode::NO_CONTENT);
        assert!(state.stream_guard.is_busy(), "la grabación sigue ocupando la única conexión (R1)");

        // Cerrar el reproductor la termina ya: proceso, permiso y carpeta.
let recording_dir = state.timeshift.lock().unwrap().as_ref().and_then(|session| session.dir().map(Path::to_path_buf));
        assert!(recording_dir.as_ref().is_some_and(|d| d.join("live.m3u8").exists()), "la grabación de esta sesión está en disco");
        let close = Request::builder().method("DELETE").uri(format!("/api/timeshift/{channel_id}")).body(Body::empty()).unwrap();
        assert_eq!(stream_router(state.clone()).oneshot(close).await.unwrap().status(), StatusCode::NO_CONTENT);
        for _ in 0..30 {
            if !state.stream_guard.is_busy() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(!state.stream_guard.is_busy(), "cerrar libera la conexión");
        assert_eq!(get_oneshot(state.clone(), &format!("/stream/timeshift/channel/{channel_id}/live.m3u8")).await.status(), StatusCode::NOT_FOUND);
        if let Some(recording_dir) = recording_dir {
            assert!(!recording_dir.exists(), "la grabación se borra del disco");
        }

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
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, &base_url, "zap").await;
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
                .oneshot(Request::builder().uri(format!("/stream/channel/{channel_id}?profile_id=1")).body(Body::empty()).unwrap())
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

    #[tokio::test]
    async fn mpv_launch_responds_service_unavailable_when_mpv_is_not_configured() {
        let db_path = temp_db_path("mpv-missing");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, "http://127.0.0.1:1", "mpv-missing").await;
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
        let fields = TitleFields { clean_title: "Peli", year: Some(2020), match_key: "peli-mpv-missing", quality: None, language: None };
        write_queries::upsert_movie(&conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
        let movie_id: i64 = conn.query_row("SELECT id FROM movies WHERE match_key = 'peli-mpv-missing'", [], |r| r.get(0)).unwrap();
        drop(conn);

        // base_state() deja mpv_path en None: exactamente el caso de esta máquina, sin
        // mpv instalado. La ruta debe fallar con un mensaje claro, no en pánico.
        let state = base_state(db_path.clone(), None, None);
        let response = state_router_oneshot(state, &format!("/stream/mpv/movie/{movie_id}?profile_id=1")).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("mpv"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn mpv_launch_returns_404_for_an_unknown_item() {
        let state = base_state(temp_db_path("mpv-404"), None, None);
        let db_path = state.db_path.clone();
        open_connection_at(&db_path).unwrap(); // solo crear el fichero con el esquema, sin ítems.
        let response = state_router_oneshot(state, "/stream/mpv/movie/9999?profile_id=1").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(&db_path);
    }

    async fn state_router_oneshot(state: StreamState, uri: &str) -> Response {
        stream_router(state).oneshot(Request::builder().method("POST").uri(uri).body(Body::empty()).unwrap()).await.unwrap()
    }

    async fn get_oneshot(state: StreamState, uri: &str) -> Response {
        stream_router(state).oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap()
    }

    fn seed_channel(db_path: &Path, provider_id: i64, remote_id: &str, has_catchup: bool, catchup_days: u32) -> i64 {
        let conn = open_connection_at(db_path).unwrap();
        let channel = RawChannel {
            remote_id: remote_id.to_string(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: None,
            epg_channel_id: None,
            has_catchup,
            catchup_days,
            sort_order: None,
        };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        write_queries::upsert_channel(&conn, provider_id, &HashMap::new(), &channel, &fields).unwrap();
        conn.query_row("SELECT id FROM channels WHERE remote_id = ?1", [remote_id], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn catchup_returns_404_for_an_unknown_channel() {
        let state = base_state(temp_db_path("catchup-404"), None, None);
        let db_path = state.db_path.clone();
        open_connection_at(&db_path).unwrap();
        let response = get_oneshot(state, "/stream/catchup/9999?profile_id=1&start=0").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn catchup_returns_404_when_the_channel_has_no_catchup_support() {
        let db_path = temp_db_path("catchup-unsupported");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, "http://127.0.0.1:1", "catchup-unsupported").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", false, 0);

        let state = base_state(db_path.clone(), None, None);
        let now = crate::db::now_epoch();
        let response = get_oneshot(state, &format!("/stream/catchup/{channel_id}?profile_id=1&start={now}")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("catch-up"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn catchup_rejects_a_start_older_than_catchup_days() {
        let db_path = temp_db_path("catchup-range");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, "http://127.0.0.1:1", "catchup-range").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", true, 3);

        let state = base_state(db_path.clone(), None, None);
        let too_old = crate::db::now_epoch() - 10 * 86_400;
        let response = get_oneshot(state, &format!("/stream/catchup/{channel_id}?profile_id=1&start={too_old}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn catchup_rejects_a_start_in_the_future() {
        let db_path = temp_db_path("catchup-future");
        let (provider_id, _keychain_guard) = seed_provider_with_credential(&db_path, "http://127.0.0.1:1", "catchup-future").await;
        let channel_id = seed_channel(&db_path, provider_id, "1", true, 3);

        let state = base_state(db_path.clone(), None, None);
        let in_the_future = crate::db::now_epoch() + 3600;
        let response = get_oneshot(state, &format!("/stream/catchup/{channel_id}?profile_id=1&start={in_the_future}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_file(&db_path);
    }
}
