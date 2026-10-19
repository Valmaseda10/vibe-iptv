//! Pausar y rebobinar el directo (F13). El mismo ffmpeg que sirve el directo graba a la
//! vez trozos HLS en disco (ver `media::ffmpeg::RemuxOptions::timeshift`), con una sola
//! conexión con el proveedor (R1). El reproductor ve el directo por la salida rápida de
//! siempre y, al pausar o rebobinar, pasa a esta grabación en el mismo segundo.
//!
//! Solo hay una grabación a la vez, como solo hay un stream a la vez. Vive mientras el
//! reproductor lea el directo o dé señales de vida (`keepalive`, o pidiendo trozos);
//! cambiar de canal o abrir otra cosa la cierra (StreamGuard) y borra su carpeta.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use super::stream::StreamState;

/// Cuánto se puede rebobinar. 30 min de un canal HD son 1–2 GB de disco.
pub const WINDOW_SECONDS: u32 = 30 * 60;
/// Sin leer el directo ni dar señales de vida durante este rato, la sesión se cierra y
/// libera la conexión con el proveedor. El reproductor manda una cada 5 s.
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);

pub type TimeshiftSlot = Arc<Mutex<Option<Arc<TimeshiftSession>>>>;

pub struct TimeshiftSession {
    channel_id: i64,
    /// `None` si no se pudo crear la carpeta: el directo sigue, sin poder pausar.
    dir: Option<PathBuf>,
    generation: u64,
    last_seen: Mutex<Instant>,
    /// El reproductor sigue leyendo la salida en directo.
    reading_live: AtomicBool,
}

impl TimeshiftSession {
    pub fn new(channel_id: i64, dir: Option<PathBuf>, generation: u64) -> Self {
        Self { channel_id, dir, generation, last_seen: Mutex::new(Instant::now()), reading_live: AtomicBool::new(true) }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Carpeta de la grabación, si se pudo crear.
    #[cfg(test)]
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Señal de vida del reproductor.
    pub fn touch(&self) {
        if let Ok(mut last_seen) = self.last_seen.lock() {
            *last_seen = Instant::now();
        }
    }

    pub fn set_reading_live(&self, reading: bool) {
        self.reading_live.store(reading, Ordering::SeqCst);
        if !reading {
            // Cuenta desde que dejó de leer, no desde la última señal de vida.
            self.touch();
        }
    }

    /// Si alguien sigue usando la sesión.
    pub fn is_wanted(&self) -> bool {
        self.reading_live.load(Ordering::SeqCst) || self.last_seen.lock().map(|t| t.elapsed() < IDLE_TIMEOUT).unwrap_or(false)
    }
}

/// Carpeta raíz de las grabaciones. Se vacía al arrancar: lo que haya es de una ejecución
/// anterior.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("vibe-iptv-timeshift")
}

pub fn clear_temp_root() {
    let _ = std::fs::remove_dir_all(temp_root());
}

/// Carpeta nueva para una grabación. Nombre aleatorio, sin nada del canal ni del
/// proveedor (R5).
pub fn new_session_dir() -> std::io::Result<PathBuf> {
    use rand::RngExt;
    let suffix: u64 = rand::rng().random();
    let dir = temp_root().join(format!("{}-{suffix:016x}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Pasa a ser la grabación vigente. La anterior ya no tiene proceso (StreamGuard lo mató
/// antes de abrir esta): se borra su carpeta.
pub fn register(slot: &TimeshiftSlot, session: Arc<TimeshiftSession>) {
    let previous = slot.lock().ok().and_then(|mut current| current.replace(session));
    if let Some(dir) = previous.and_then(|p| p.dir.clone()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Al terminar una sesión: si sigue siendo la vigente, se quita y se borra su carpeta.
pub fn finish(slot: &TimeshiftSlot, session: &Arc<TimeshiftSession>) {
    let Ok(mut current) = slot.lock() else { return };
    if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, session)) {
        current.take();
        if let Some(dir) = &session.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn current_for(slot: &TimeshiftSlot, channel_id: i64) -> Option<Arc<TimeshiftSession>> {
    slot.lock().ok()?.as_ref().filter(|s| s.channel_id == channel_id).cloned()
}

/// Solo nombres que ffmpeg genera aquí: nada de rutas.
fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && [".m3u8", ".m4s", ".mp4"].iter().any(|ext| name.ends_with(ext))
}

fn content_type(name: &str) -> &'static str {
    if name.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if name.ends_with(".m4s") {
        "video/iso.segment"
    } else {
        "video/mp4"
    }
}

/// `GET /stream/timeshift/channel/:id/:file` — la lista HLS (`live.m3u8`), su `init.mp4`
/// y sus trozos, de la grabación vigente si es de ese canal. Cada petición cuenta como
/// señal de vida. No toca al proveedor.
pub async fn get_file(State(state): State<StreamState>, AxumPath((channel_id, file)): AxumPath<(i64, String)>) -> Response {
    if !is_safe_file_name(&file) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(session) = current_for(&state.timeshift, channel_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    session.touch();
    let Some(dir) = session.dir.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(Path::new(&dir).join(&file)).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, content_type(&file)), (header::CACHE_CONTROL, "no-store")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /api/timeshift/:channel_id/keepalive` — el reproductor sigue en este canal
/// (aunque esté en pausa y no lea nada). 404 si ya no hay grabación de ese canal.
pub async fn post_keepalive(State(state): State<StreamState>, AxumPath(channel_id): AxumPath<i64>) -> StatusCode {
    match current_for(&state.timeshift, channel_id) {
        Some(session) => {
            session.touch();
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

/// `DELETE /api/timeshift/:channel_id` — el reproductor se cierra: la conexión con el
/// proveedor se libera ya, sin esperar a que caduquen las señales de vida.
pub async fn delete_session(State(state): State<StreamState>, AxumPath(channel_id): AxumPath<i64>) -> StatusCode {
    if let Some(session) = current_for(&state.timeshift, channel_id) {
        state.stream_guard.release_process(session.generation()).await;
        finish(&state.timeshift, &session);
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_hls_file_names_are_served() {
        for ok in ["live.m3u8", "init.mp4", "seg-000012.m4s"] {
            assert!(is_safe_file_name(ok), "{ok}");
        }
        for bad in ["../live.m3u8", "..\\x.m4s", "a/b.m4s", "secret.txt", "", "live.m3u8.tmp"] {
            assert!(!is_safe_file_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_session_is_wanted_while_read_or_recently_touched() {
        let session = TimeshiftSession::new(1, None, 1);
        assert!(session.is_wanted());
        session.set_reading_live(false);
        assert!(session.is_wanted(), "recién pausado: dentro del margen");
        *session.last_seen.lock().unwrap() = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        assert!(!session.is_wanted());
        session.touch();
        assert!(session.is_wanted());
    }

    #[test]
    fn registering_a_new_session_deletes_the_previous_recording() {
        let slot: TimeshiftSlot = Default::default();
        let first_dir = new_session_dir().unwrap();
        std::fs::write(first_dir.join("live.m3u8"), "#EXTM3U").unwrap();
        let first = Arc::new(TimeshiftSession::new(1, Some(first_dir.clone()), 1));
        register(&slot, Arc::clone(&first));

        let second_dir = new_session_dir().unwrap();
        let second = Arc::new(TimeshiftSession::new(2, Some(second_dir.clone()), 2));
        register(&slot, Arc::clone(&second));
        assert!(!first_dir.exists(), "la grabación del canal anterior se borra");

        finish(&slot, &first); // llega tarde: ya no es la vigente, no toca nada.
        assert!(second_dir.exists());
        finish(&slot, &second);
        assert!(!second_dir.exists());
        assert!(slot.lock().unwrap().is_none());
    }
}
