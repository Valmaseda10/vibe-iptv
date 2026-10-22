//! Pausar y rebobinar el directo en iPhone y iPad (F15), sin ffmpeg.
//!
//! En el PC lo hace ffmpeg (`server::timeshift`, F13). En iOS no hay ffmpeg, así que el
//! núcleo graba los bytes del proveedor tal cual (MPEG-TS) en un fichero circular y VLC
//! lee de esa grabación en vez de del proveedor:
//!
//! - **Pausa:** VLC deja de leer, la grabación sigue. Al reanudar continúa justo donde
//!   estaba, ya por detrás del directo.
//! - **Rebobinar, adelantar, volver al directo:** VLC vuelve a pedir el stream con
//!   `behind_s` (segundos por detrás del directo). Los segundos se traducen a bytes con
//!   la tasa media medida de la grabación; MPEG-TS es seguible por bytes (paquetes de
//!   188), así que VLC se resincroniza solo.
//!
//! Una sola conexión con el proveedor (R1): la grabación sostiene el permiso del
//! `StreamGuard` y termina en cuanto otra sesión lo pide, al cerrarse el reproductor
//! (`DELETE /api/live-buffer`) o si nadie la lee durante un rato.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Notify;

use crate::util::stream_guard::{StreamGuard, StreamPermit, UpstreamToken};

/// Paquete MPEG-TS: los saltos caen en múltiplos para no partir uno por la mitad.
const TS_PACKET: u64 = 188;
/// Lo que se lee de una vez para mandarlo a VLC.
const READ_CHUNK: u64 = 64 * 1024;
/// Margen respecto a lo más antiguo de la grabación: el escritor no debe pisar lo que se
/// está leyendo.
const SAFETY_MARGIN: u64 = 4 * READ_CHUNK;
/// Tasa supuesta mientras no se ha medido la real (5 Mbit/s, un HD típico de IPTV).
const FALLBACK_BYTES_PER_SECOND: f64 = 625_000.0;

/// Tamaño y paciencia de la grabación. Los tests usan valores pequeños.
#[derive(Clone, Copy, Debug)]
pub struct LiveBufferConfig {
    /// Bytes del fichero circular: 600 MB son unos 15 minutos de un canal HD.
    pub capacity: u64,
    /// Sin nadie leyendo durante este tiempo, la grabación termina (y suelta la conexión
    /// con el proveedor). Cubre de sobra lo que tarda VLC en reabrir tras un salto.
    pub idle_timeout: Duration,
}

impl Default for LiveBufferConfig {
    fn default() -> Self {
        Self { capacity: 600 * 1024 * 1024, idle_timeout: Duration::from_secs(30) }
    }
}

/// Sesiones cerradas que se recuerdan (las últimas): de sobra para las reconexiones tardías.
const CLOSED_SESSIONS_KEPT: usize = 64;

/// La grabación en curso (como mucho una), compartida por las rutas.
#[derive(Clone, Default)]
pub struct LiveBufferSlot {
    current: Arc<Mutex<Option<Arc<Recording>>>>,
    config: Arc<Mutex<Option<LiveBufferConfig>>>,
    /// Sesiones de reproducción ya cerradas (`?session=` de la URL): ver `close_session`.
    closed_sessions: Arc<Mutex<std::collections::VecDeque<String>>>,
}

impl LiveBufferSlot {
    #[cfg(test)]
    pub fn with_config(config: LiveBufferConfig) -> Self {
        let slot = Self::default();
        if let Ok(mut current) = slot.config.lock() {
            *current = Some(config);
        }
        slot
    }

    fn config(&self) -> LiveBufferConfig {
        self.config.lock().ok().and_then(|config| *config).unwrap_or_default()
    }

    /// La grabación de `channel_id`, si existe y sigue viva y vigente.
    pub fn existing(&self, channel_id: i64, guard: &StreamGuard) -> Option<Arc<Recording>> {
        let current = self.current.lock().ok()?;
        current
            .as_ref()
            .filter(|rec| rec.channel_id == channel_id && !rec.finished.load(Ordering::SeqCst) && guard.is_current_generation(rec.generation))
            .cloned()
    }

    /// Empieza a grabar `upstream` (la respuesta del proveedor ya abierta con `permit`).
    /// Sustituye a la grabación anterior, si la había.
    pub async fn start<S>(&self, channel_id: i64, upstream: S, permit: StreamPermit, guard: Arc<StreamGuard>) -> std::io::Result<Arc<Recording>>
    where
        S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    {
        let config = self.config();
        let path = new_temp_file()?;
        let file = tokio::fs::OpenOptions::new().create(true).truncate(true).write(true).read(true).open(&path).await?;
        let recording = Arc::new(Recording {
            channel_id,
            path,
            capacity: config.capacity,
            idle_timeout: config.idle_timeout,
            written: AtomicU64::new(0),
            first_byte_at: Mutex::new(None),
            readers: AtomicUsize::new(0),
            idle_since: Mutex::new(Some(Instant::now())),
            finished: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            notify: Notify::new(),
            generation: permit.generation(),
        });
        if let Ok(mut current) = self.current.lock() {
            if let Some(previous) = current.replace(Arc::clone(&recording)) {
                previous.stop_requested.store(true, Ordering::SeqCst);
            }
        }
        let token = guard.upstream_opened();
        tokio::spawn(record(Arc::clone(&recording), file, upstream, permit, guard, token));
        Ok(recording)
    }

    /// El reproductor que usaba esta sesión se cerró: sus peticiones ya no abren nada. VLC
    /// (`--http-reconnect`) volvía a pedir la URL al cortarse la grabación aunque se
    /// estuviera cerrando, y el directo quedaba abierto con el proveedor sin nadie mirando.
    pub fn close_session(&self, session: &str) {
        if let Ok(mut closed) = self.closed_sessions.lock() {
            if !closed.iter().any(|s| s == session) {
                closed.push_back(session.to_string());
                while closed.len() > CLOSED_SESSIONS_KEPT {
                    closed.pop_front();
                }
            }
        }
    }

    /// `true` si esa sesión de reproducción ya se cerró.
    pub fn session_closed(&self, session: &str) -> bool {
        self.closed_sessions.lock().map(|closed| closed.iter().any(|s| s == session)).unwrap_or(false)
    }

    /// Termina la grabación en curso ya (el reproductor se cerró). Con `channel_id`, solo
    /// si es la de ese canal: un aviso de cierre que llega tarde no corta el canal que se
    /// abrió después.
    pub fn stop(&self, channel_id: Option<i64>) {
        if let Ok(mut current) = self.current.lock() {
            if channel_id.is_some_and(|id| current.as_ref().is_some_and(|rec| rec.channel_id != id)) {
                return;
            }
            if let Some(recording) = current.take() {
                recording.stop_requested.store(true, Ordering::SeqCst);
                recording.notify.notify_waiters();
            }
        }
    }
}

/// Una grabación del directo en un fichero circular.
pub struct Recording {
    channel_id: i64,
    path: PathBuf,
    capacity: u64,
    idle_timeout: Duration,
    /// Bytes recibidos en total (posición lógica del final de la grabación).
    written: AtomicU64,
    first_byte_at: Mutex<Option<Instant>>,
    readers: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
    finished: AtomicBool,
    stop_requested: AtomicBool,
    notify: Notify,
    generation: u64,
}

impl Recording {
    /// Lo más antiguo que se puede leer todavía sin que el escritor lo pise.
    fn oldest(&self) -> u64 {
        let written = self.written.load(Ordering::SeqCst);
        if written > self.capacity {
            written - self.capacity + SAFETY_MARGIN.min(self.capacity / 4)
        } else {
            0
        }
    }

    /// `pos` llevado al primer inicio de paquete que no esté antes de lo más antiguo.
    fn first_packet_from(&self, pos: u64) -> u64 {
        let pos = pos.max(self.oldest());
        pos.div_ceil(TS_PACKET) * TS_PACKET
    }

    fn bytes_per_second(&self) -> f64 {
        let written = self.written.load(Ordering::SeqCst) as f64;
        let elapsed = self.first_byte_at.lock().ok().and_then(|at| *at).map(|at| at.elapsed().as_secs_f64()).unwrap_or(0.0);
        if elapsed >= 2.0 && written > 0.0 {
            written / elapsed
        } else {
            FALLBACK_BYTES_PER_SECOND
        }
    }

    /// Posición lógica a `behind_s` segundos del directo, dentro de lo grabado.
    pub fn position_behind(&self, behind_s: f64) -> u64 {
        let written = self.written.load(Ordering::SeqCst);
        let back = (behind_s.max(0.0) * self.bytes_per_second()) as u64;
        let target = written.saturating_sub(back);
        let aligned = target - target % TS_PACKET;
        if aligned < self.oldest() {
            self.first_packet_from(aligned)
        } else {
            aligned
        }
    }

    /// Cuerpo de respuesta para VLC: desde `behind_s` segundos atrás, siguiendo la
    /// grabación a medida que crece, hasta que esta termine.
    pub fn reader(self: &Arc<Self>, behind_s: f64) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
        let state = ReaderState { recording: Arc::clone(self), file: None, pos: self.position_behind(behind_s), _attached: ReaderAttached::new(Arc::clone(self)) };
        futures_util::stream::unfold(state, |mut state| async move {
            loop {
                let rec = Arc::clone(&state.recording);
                let oldest = rec.oldest();
                if state.pos < oldest {
                    // Se quedó tan atrás que lo pisó la grabación: salta a lo más antiguo.
                    state.pos = rec.first_packet_from(oldest);
                }
                let written = rec.written.load(Ordering::SeqCst);
                if state.pos < written {
                    let physical = state.pos % rec.capacity;
                    let len = READ_CHUNK.min(written - state.pos).min(rec.capacity - physical);
                    if state.file.is_none() {
                        match tokio::fs::File::open(&rec.path).await {
                            Ok(file) => state.file = Some(file),
                            Err(err) => return Some((Err(err), state)),
                        }
                    }
                    let Some(file) = state.file.as_mut() else { return None };
                    let mut buf = vec![0u8; len as usize];
                    let read = async {
                        file.seek(std::io::SeekFrom::Start(physical)).await?;
                        file.read_exact(&mut buf).await
                    }
                    .await;
                    if let Err(err) = read {
                        return Some((Err(err), state));
                    }
                    if state.pos < rec.oldest() {
                        continue; // lo pisó mientras se leía: se descarta y se vuelve a situar.
                    }
                    state.pos += len;
                    return Some((Ok(Bytes::from(buf)), state));
                }
                if rec.finished.load(Ordering::SeqCst) {
                    return None;
                }
                let _ = tokio::time::timeout(Duration::from_millis(250), rec.notify.notified()).await;
            }
        })
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct ReaderState {
    recording: Arc<Recording>,
    file: Option<tokio::fs::File>,
    pos: u64,
    _attached: ReaderAttached,
}

/// Cuenta los lectores de una grabación: sin ninguno durante `idle_timeout`, termina.
struct ReaderAttached(Arc<Recording>);

impl ReaderAttached {
    fn new(recording: Arc<Recording>) -> Self {
        recording.readers.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut idle) = recording.idle_since.lock() {
            *idle = None;
        }
        Self(recording)
    }
}

impl Drop for ReaderAttached {
    fn drop(&mut self) {
        if self.0.readers.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Ok(mut idle) = self.0.idle_since.lock() {
                *idle = Some(Instant::now());
            }
        }
    }
}

async fn record<S>(rec: Arc<Recording>, mut file: tokio::fs::File, upstream: S, permit: StreamPermit, guard: Arc<StreamGuard>, token: UpstreamToken)
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    let _token = token; // se suelta el último: `acquire()` espera a que la conexión se cierre.
    let mut upstream = upstream;
    let mut replaced = guard.watch_generation();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = StreamGuard::replaced(&mut replaced, permit.generation()) => break,
            chunk = upstream.next() => match chunk {
                Some(Ok(bytes)) => {
                    if let Err(err) = write_ring(&mut file, &rec, &bytes).await {
                        tracing::warn!(error = %err, "no se pudo escribir la grabación del directo");
                        break;
                    }
                    if let Ok(mut first) = rec.first_byte_at.lock() {
                        first.get_or_insert_with(Instant::now);
                    }
                    rec.written.fetch_add(bytes.len() as u64, Ordering::SeqCst);
                    rec.notify.notify_waiters();
                }
                Some(Err(_)) | None => {
                    tracing::info!(channel_id = rec.channel_id, "el proveedor cerró el directo que se grababa");
                    break;
                }
            },
            _ = tick.tick() => {}
        }
        if rec.stop_requested.load(Ordering::SeqCst) || !guard.is_current_generation(permit.generation()) {
            break;
        }
        let idle_for = rec.idle_since.lock().ok().and_then(|idle| *idle).map(|since| since.elapsed());
        if rec.readers.load(Ordering::SeqCst) == 0 && idle_for.is_some_and(|idle| idle >= rec.idle_timeout) {
            break;
        }
    }
    drop(upstream); // cierra la conexión con el proveedor
    rec.finished.store(true, Ordering::SeqCst);
    rec.notify.notify_waiters();
    drop(permit); // libera `is_busy()`: la sincronización del catálogo puede seguir.
}

async fn write_ring(file: &mut tokio::fs::File, rec: &Recording, bytes: &[u8]) -> std::io::Result<()> {
    let mut pos = rec.written.load(Ordering::SeqCst);
    let mut rest = bytes;
    while !rest.is_empty() {
        let physical = pos % rec.capacity;
        let room = (rec.capacity - physical) as usize;
        let (now, later) = rest.split_at(room.min(rest.len()));
        file.seek(std::io::SeekFrom::Start(physical)).await?;
        file.write_all(now).await?;
        pos += now.len() as u64;
        rest = later;
    }
    file.flush().await
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("vibe-iptv-livebuffer")
}

fn new_temp_file() -> std::io::Result<PathBuf> {
    let dir = temp_root();
    std::fs::create_dir_all(&dir)?;
    use rand::RngExt;
    let suffix: u64 = rand::rng().random();
    Ok(dir.join(format!("{}-{suffix}.ts", std::process::id())))
}

/// Borra grabaciones que quedaran de una ejecución anterior (cierre a la fuerza).
pub fn clear_temp_root() {
    let _ = std::fs::remove_dir_all(temp_root());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_stream(chunks: Vec<Vec<u8>>, every: Duration) -> impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static {
        Box::pin(futures_util::stream::iter(chunks).then(move |chunk| async move {
            tokio::time::sleep(every).await;
            Ok::<_, reqwest::Error>(Bytes::from(chunk))
        }))
    }

    fn packet(n: u8) -> Vec<u8> {
        vec![n; TS_PACKET as usize]
    }

    async fn wait_written(rec: &Recording, at_least: u64) {
        for _ in 0..200 {
            if rec.written.load(Ordering::SeqCst) >= at_least {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("la grabación no llegó a {at_least} bytes");
    }

    #[tokio::test]
    async fn a_paused_reader_resumes_where_it_was_while_recording_continues() {
        let guard = StreamGuard::new();
        let slot = LiveBufferSlot::with_config(LiveBufferConfig { capacity: 1_000 * TS_PACKET, idle_timeout: Duration::from_secs(5) });
        let chunks: Vec<Vec<u8>> = (0..20).map(packet).collect();
        let permit = guard.acquire().await;
        let rec = slot.start(7, bytes_stream(chunks, Duration::from_millis(15)), permit, Arc::clone(&guard)).await.unwrap();
        wait_written(&rec, 3 * TS_PACKET).await;

        // Empieza en el directo (behind 0): lo siguiente que llegue.
        let mut reader = Box::pin(rec.reader(0.0));
        let first = reader.next().await.unwrap().unwrap();
        let first_value = first[0];
        // "Pausa": no se lee mientras llegan más paquetes.
        wait_written(&rec, 20 * TS_PACKET).await;
        let mut rest = Vec::new();
        while let Some(chunk) = reader.next().await {
            rest.extend_from_slice(&chunk.unwrap());
        }
        // Al reanudar sigue justo después de lo que ya había dado, sin saltos.
        assert_eq!(rest.first().copied(), Some(first_value.wrapping_add((first.len() / TS_PACKET as usize) as u8)));
        assert_eq!(rest.last().copied(), Some(19));
        assert!(rec.finished.load(Ordering::SeqCst), "el proveedor cerró: la grabación termina");
    }

    #[tokio::test]
    async fn behind_seconds_start_further_back_and_the_ring_wraps() {
        let guard = StreamGuard::new();
        // Anillo de 10 paquetes: tras 30 paquetes solo quedan los últimos (menos el margen).
        let slot = LiveBufferSlot::with_config(LiveBufferConfig { capacity: 10 * TS_PACKET, idle_timeout: Duration::from_secs(5) });
        let chunks: Vec<Vec<u8>> = (0..30).map(packet).collect();
        let permit = guard.acquire().await;
        let rec = slot.start(7, bytes_stream(chunks, Duration::from_millis(1)), permit, Arc::clone(&guard)).await.unwrap();
        wait_written(&rec, 30 * TS_PACKET).await;

        // Muy atrás: se queda en lo más antiguo que sigue grabado, alineado a paquete.
        let pos = rec.position_behind(10_000.0);
        assert_eq!(pos % TS_PACKET, 0);
        assert!(pos >= 20 * TS_PACKET, "{pos}");
        let mut reader = Box::pin(rec.reader(10_000.0));
        let mut all = Vec::new();
        while let Some(chunk) = reader.next().await {
            all.extend_from_slice(&chunk.unwrap());
        }
        assert!(!all.is_empty());
        assert_eq!(all.len() as u64 % TS_PACKET, 0);
        assert_eq!(*all.last().unwrap(), 29, "el final es lo último grabado");
        assert!(all[0] >= 20, "no puede salir nada ya pisado por el anillo: {}", all[0]);
    }

    #[tokio::test]
    async fn another_stream_session_ends_the_recording_and_frees_the_guard() {
        let guard = StreamGuard::new();
        let slot = LiveBufferSlot::with_config(LiveBufferConfig { capacity: 1_000 * TS_PACKET, idle_timeout: Duration::from_secs(60) });
        let chunks: Vec<Vec<u8>> = (0..200).map(|i| packet(i as u8)).collect();
        let permit = guard.acquire().await;
        let rec = slot.start(7, bytes_stream(chunks, Duration::from_millis(20)), permit, Arc::clone(&guard)).await.unwrap();
        wait_written(&rec, TS_PACKET).await;
        assert!(slot.existing(7, &guard).is_some());

        // Otro canal (o cualquier otro stream) pide la conexión: la grabación se corta.
        let _other = guard.acquire().await;
        for _ in 0..100 {
            if rec.finished.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(rec.finished.load(Ordering::SeqCst));
        assert!(slot.existing(7, &guard).is_none());
    }

    #[tokio::test]
    async fn closing_another_channel_does_not_end_the_current_recording() {
        let guard = StreamGuard::new();
        let slot = LiveBufferSlot::with_config(LiveBufferConfig { capacity: 1_000 * TS_PACKET, idle_timeout: Duration::from_secs(60) });
        let chunks: Vec<Vec<u8>> = (0..200).map(|i| packet(i as u8)).collect();
        let permit = guard.acquire().await;
        let rec = slot.start(7, bytes_stream(chunks, Duration::from_millis(20)), permit, Arc::clone(&guard)).await.unwrap();
        wait_written(&rec, TS_PACKET).await;

        // El aviso de cierre del canal 3 llega tarde, con el 7 ya abierto: no lo toca.
        slot.stop(Some(3));
        assert!(slot.existing(7, &guard).is_some());
        assert!(!rec.stop_requested.load(Ordering::SeqCst));

        // El del propio canal sí lo termina.
        slot.stop(Some(7));
        assert!(rec.stop_requested.load(Ordering::SeqCst));
        assert!(slot.existing(7, &guard).is_none());
    }

    #[test]
    fn a_closed_session_stays_closed_and_only_the_last_ones_are_kept() {
        let slot = LiveBufferSlot::default();
        assert!(!slot.session_closed("a"));
        slot.close_session("a");
        slot.close_session("a");
        assert!(slot.session_closed("a"));
        assert!(!slot.session_closed("b"));
        for i in 0..CLOSED_SESSIONS_KEPT {
            slot.close_session(&format!("s{i}"));
        }
        assert!(!slot.session_closed("a"), "la más vieja se olvida");
        assert!(slot.session_closed(&format!("s{}", CLOSED_SESSIONS_KEPT - 1)));
    }

    #[tokio::test]
    async fn nobody_reading_for_a_while_ends_the_recording() {
        let guard = StreamGuard::new();
        let slot = LiveBufferSlot::with_config(LiveBufferConfig { capacity: 1_000 * TS_PACKET, idle_timeout: Duration::from_millis(300) });
        let chunks: Vec<Vec<u8>> = (0..500).map(|i| packet(i as u8)).collect();
        let permit = guard.acquire().await;
        let rec = slot.start(7, bytes_stream(chunks, Duration::from_millis(10)), permit, Arc::clone(&guard)).await.unwrap();
        {
            let mut reader = Box::pin(rec.reader(0.0));
            let _ = reader.next().await;
        } // el lector se va (VLC cerrado)
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        assert!(rec.finished.load(Ordering::SeqCst), "sin lectores tiene que soltar la conexión");
        assert!(!guard.is_busy(), "y liberar el StreamGuard para que siga la sincronización");
    }
}
