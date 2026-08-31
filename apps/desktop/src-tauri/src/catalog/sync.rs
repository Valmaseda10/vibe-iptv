//! Sincronizador de catálogo. Pasos 1 a 4 de `docs/ARCHITECTURE.md`, sección
//! "Sincronización", con el paso 5 (normalización) integrado en cada upsert. El paso 6
//! (TMDB) lo dispara `commands::spawn_sync` después de que esta función termine, no vive
//! aquí. Los pasos 7 y 8 (imágenes ya las sirve `server::routes`, EPG llega en F7).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::Serialize;
use ts_rs::TS;

use crate::catalog::normalize::{self, NormalizedTitle};
use crate::db::{self, queries};
use crate::sources::{MediaKind, Source, SourceError};
use crate::util::stream_guard::StreamGuard;

const BATCH_SIZE: usize = 500;
const PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("error del proveedor: {0}")]
    Source(#[from] SourceError),
    #[error("error de base de datos: {0}")]
    Db(#[from] db::DbError),
    #[error("tarea interna de base de datos interrumpida: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("sincronización cancelada")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, SyncError>;

/// Progreso emitido al frontend por eventos de Tauri (`sync://progress`).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SyncProgress {
    // Ver el comentario en ProviderSummary::id: Tauri manda esto por JSON normal, así
    // que en el frontend nunca es un bigint de verdad.
    #[ts(type = "number")]
    pub provider_id: i64,
    pub stage: String,
    pub current: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct SyncSummary {
    #[ts(type = "number")]
    pub provider_id: i64,
    #[ts(type = "number")]
    pub categories: i64,
    #[ts(type = "number")]
    pub channels: i64,
    #[ts(type = "number")]
    pub movies: i64,
    #[ts(type = "number")]
    pub series: i64,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

/// Orquesta la sincronización completa de un proveedor. `conn` viaja entre el hilo async
/// (llamadas a `source`, que son de red) y el pool de tareas bloqueantes de tokio
/// (escritura en SQLite, que es síncrona): `rusqlite::Connection` no es async.
pub async fn run(
    mut conn: Connection,
    provider_id: i64,
    source: Arc<dyn Source + Send + Sync>,
    stream_guard: Arc<StreamGuard>,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(SyncProgress),
) -> Result<(Connection, SyncSummary)> {
    let start = Instant::now();

    // Paso 1: cuenta y max_connections.
    let account = source.account_info().await?;
    if !account.auth_ok {
        return Err(SyncError::Source(SourceError::AuthFailed));
    }
    let max_connections = account.max_connections as i64;
    let expires_at = account.expires_at;
    let (next_conn, ()) = run_blocking(conn, move |conn| {
        queries::update_provider_account_status(conn, provider_id, max_connections, expires_at)
    })
    .await?;
    conn = next_conn;

    // Paso 2: categorías, una llamada por tipo. El mapa remote_id->id local hace falta
    // para enlazar canales y películas con su categoría en el paso siguiente.
    let live_categories = source.categories(MediaKind::Live).await?;
    let movie_categories = source.categories(MediaKind::Movie).await?;
    let series_categories = source.categories(MediaKind::Series).await?;
    let total_categories = (live_categories.len() + movie_categories.len() + series_categories.len()) as i64;

    let (next_conn, live_map) =
        run_blocking(conn, move |conn| queries::upsert_categories(conn, provider_id, MediaKind::Live, &live_categories)).await?;
    conn = next_conn;
    let (next_conn, movie_map) =
        run_blocking(conn, move |conn| queries::upsert_categories(conn, provider_id, MediaKind::Movie, &movie_categories)).await?;
    conn = next_conn;
    let (next_conn, series_map) =
        run_blocking(conn, move |conn| queries::upsert_categories(conn, provider_id, MediaKind::Series, &series_categories)).await?;
    conn = next_conn;
    on_progress(SyncProgress { provider_id, stage: "categorias".into(), current: 1, total: 1 });

    // Paso 3: canales de directo, por lotes de 500.
    let channels = source.live_channels().await?;
    let total_channels = channels.len() as u32;
    for (batch_index, chunk) in channels.chunks(BATCH_SIZE).enumerate() {
        wait_while_streaming(&stream_guard, &cancel).await?;
        let chunk = chunk.to_vec();
        let map = live_map.clone();
        let (next_conn, ()) = run_blocking(conn, move |conn| {
            let tx = conn.unchecked_transaction()?;
            for channel in &chunk {
                let (normalized, match_key) = normalize_title(&channel.raw_name);
                let fields = title_fields(&normalized, &match_key);
                queries::upsert_channel(&tx, provider_id, &map, channel, &fields)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        conn = next_conn;
        report_batch_progress(&mut on_progress, provider_id, "canales", batch_index, total_channels);
    }
    if total_channels == 0 {
        on_progress(SyncProgress { provider_id, stage: "canales".into(), current: 0, total: 0 });
    }

    // Paso 4a: películas.
    let movies = source.movies().await?;
    let total_movies = movies.len() as u32;
    for (batch_index, chunk) in movies.chunks(BATCH_SIZE).enumerate() {
        wait_while_streaming(&stream_guard, &cancel).await?;
        let chunk = chunk.to_vec();
        let map = movie_map.clone();
        let (next_conn, ()) = run_blocking(conn, move |conn| {
            let tx = conn.unchecked_transaction()?;
            for movie in &chunk {
                let (normalized, match_key) = normalize_title(&movie.raw_title);
                let fields = title_fields(&normalized, &match_key);
                queries::upsert_movie(&tx, provider_id, &map, movie, &fields)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        conn = next_conn;
        report_batch_progress(&mut on_progress, provider_id, "peliculas", batch_index, total_movies);
    }
    if total_movies == 0 {
        on_progress(SyncProgress { provider_id, stage: "peliculas".into(), current: 0, total: 0 });
    }

    // Paso 4b: series (sin detalle: get_series_info es bajo demanda, ver docs/XTREAM-API.md).
    let series = source.series().await?;
    let total_series = series.len() as u32;
    for (batch_index, chunk) in series.chunks(BATCH_SIZE).enumerate() {
        wait_while_streaming(&stream_guard, &cancel).await?;
        let chunk = chunk.to_vec();
        let map = series_map.clone();
        let (next_conn, ()) = run_blocking(conn, move |conn| {
            let tx = conn.unchecked_transaction()?;
            for one_series in &chunk {
                let (normalized, match_key) = normalize_title(&one_series.raw_title);
                let fields = title_fields(&normalized, &match_key);
                queries::upsert_series(&tx, provider_id, &map, one_series, &fields)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        conn = next_conn;
        report_batch_progress(&mut on_progress, provider_id, "series", batch_index, total_series);
    }
    if total_series == 0 {
        on_progress(SyncProgress { provider_id, stage: "series".into(), current: 0, total: 0 });
    }

    let (next_conn, ()) = run_blocking(conn, move |conn| queries::mark_provider_synced(conn, provider_id)).await?;
    conn = next_conn;

    // Recalcular match_key con normalización real puede dejar huérfanas filas de
    // sincronizaciones anteriores (p. ej. el placeholder de F2, o un match_key que
    // cambia porque el título crudo del proveedor cambió). Ver docs/DECISIONS.md, F3.
    let (next_conn, _) = run_blocking(conn, queries::delete_orphaned_movies_and_series).await?;
    conn = next_conn;

    // F4 (búsqueda por HTTP) depende de esto; se reconstruye entera aquí, no con
    // triggers por fila (ver el comentario de rebuild_search_index).
    let (next_conn, ()) = run_blocking(conn, queries::rebuild_search_index).await?;
    conn = next_conn;

    let (channels_count, movies_count, series_count) = {
        let channels = queries::count_channels(&conn, provider_id)?;
        let movies = queries::count_movie_sources(&conn, provider_id)?;
        let series = queries::count_series_sources(&conn, provider_id)?;
        (channels, movies, series)
    };

    let summary = SyncSummary {
        provider_id,
        categories: total_categories,
        channels: channels_count,
        movies: movies_count,
        series: series_count,
        duration_ms: start.elapsed().as_millis() as u64,
    };

    Ok((conn, summary))
}

fn normalize_title(raw: &str) -> (NormalizedTitle, String) {
    let normalized = normalize::normalize(raw);
    let match_key = normalize::match_key(&normalized.clean_title, normalized.year);
    (normalized, match_key)
}

fn title_fields<'a>(normalized: &'a NormalizedTitle, match_key: &'a str) -> queries::TitleFields<'a> {
    queries::TitleFields {
        clean_title: &normalized.clean_title,
        year: normalized.year,
        match_key,
        quality: normalized.quality.as_deref(),
        language: normalized.language.as_deref(),
    }
}

fn report_batch_progress(
    on_progress: &mut impl FnMut(SyncProgress),
    provider_id: i64,
    stage: &str,
    batch_index: usize,
    total: u32,
) {
    let done = (((batch_index + 1) * BATCH_SIZE) as u32).min(total);
    on_progress(SyncProgress { provider_id, stage: stage.to_string(), current: done, total });
}

/// Se pausa (nunca cancela) mientras haya una sesión de stream activa (regla R1). Sí
/// respeta una cancelación explícita pedida por el usuario.
async fn wait_while_streaming(stream_guard: &Arc<StreamGuard>, cancel: &Arc<AtomicBool>) -> Result<()> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(SyncError::Cancelled);
        }
        if !stream_guard.is_busy() {
            return Ok(());
        }
        tokio::time::sleep(PAUSE_POLL_INTERVAL).await;
    }
}

/// Mueve `conn` al pool de tareas bloqueantes de tokio, ejecuta `f`, y la devuelve junto
/// con el resultado. rusqlite es síncrono; esto evita bloquear el runtime async mientras
/// se escribe.
async fn run_blocking<T, F>(conn: Connection, f: F) -> Result<(Connection, T)>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> db::Result<T> + Send + 'static,
{
    let (conn, result) = tokio::task::spawn_blocking(move || {
        let result = f(&conn);
        (conn, result)
    })
    .await?;
    Ok((conn, result?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{self, ProviderKind};
    use crate::db::schema::open_connection_at;
    use crate::sources::{AccountInfo, RawCategory, RawChannel, RawMovie, RawSeries, RawSeriesDetail, StreamRef, XmltvStream};
    use async_trait::async_trait;
    use std::path::Path;

    struct FakeSource {
        account: AccountInfo,
        channels: Vec<RawChannel>,
        movies: Vec<RawMovie>,
        series: Vec<RawSeries>,
    }

    impl FakeSource {
        fn with_counts(channels: usize, movies: usize, series: usize) -> Self {
            Self {
                account: AccountInfo {
                    auth_ok: true,
                    status: "Active".into(),
                    expires_at: Some(1893456000),
                    max_connections: 2,
                    active_connections: Some(1),
                },
                channels: (0..channels)
                    .map(|i| RawChannel {
                        remote_id: i.to_string(),
                        category_remote_id: None,
                        raw_name: format!("Canal {i}"),
                        logo_url: None,
                        epg_channel_id: None,
                        has_catchup: false,
                        catchup_days: 0,
                        sort_order: None,
                    })
                    .collect(),
                movies: (0..movies)
                    .map(|i| RawMovie {
                        remote_id: i.to_string(),
                        category_remote_id: None,
                        raw_title: format!("Película {i}"),
                        container_extension: Some("mkv".into()),
                        logo_url: None,
                        added_at: None,
                        rating: None,
                        tmdb_id: None,
                    })
                    .collect(),
                series: (0..series)
                    .map(|i| RawSeries {
                        remote_id: i.to_string(),
                        raw_title: format!("Serie {i}"),
                        category_remote_id: None,
                        cover_url: None,
                        plot: None,
                        release_date: None,
                    })
                    .collect(),
            }
        }

        fn auth_failed() -> Self {
            Self {
                account: AccountInfo { auth_ok: false, status: "Disabled".into(), expires_at: None, max_connections: 1, active_connections: None },
                ..Self::with_counts(0, 0, 0)
            }
        }
    }

    #[async_trait]
    impl Source for FakeSource {
        async fn account_info(&self) -> crate::sources::Result<AccountInfo> {
            Ok(self.account.clone())
        }
        async fn categories(&self, _kind: MediaKind) -> crate::sources::Result<Vec<RawCategory>> {
            Ok(vec![])
        }
        async fn live_channels(&self) -> crate::sources::Result<Vec<RawChannel>> {
            Ok(self.channels.clone())
        }
        async fn movies(&self) -> crate::sources::Result<Vec<RawMovie>> {
            Ok(self.movies.clone())
        }
        async fn series(&self) -> crate::sources::Result<Vec<RawSeries>> {
            Ok(self.series.clone())
        }
        async fn series_detail(&self, _id: &str) -> crate::sources::Result<RawSeriesDetail> {
            Ok(RawSeriesDetail { episodes: vec![] })
        }
        async fn epg_xmltv(&self) -> crate::sources::Result<Option<XmltvStream>> {
            Ok(None)
        }
        fn stream_url(&self, _item: &StreamRef) -> crate::sources::Result<reqwest::Url> {
            unimplemented!("no usado en estos tests")
        }
    }

    fn memory_conn() -> Connection {
        open_connection_at(Path::new(":memory:")).unwrap()
    }

    fn provider_id(conn: &Connection) -> i64 {
        queries::insert_provider(conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap()
    }

    #[tokio::test]
    async fn sync_populates_expected_counts() {
        let conn = memory_conn();
        let id = provider_id(&conn);
        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeSource::with_counts(3, 2, 1));

        let (_conn, summary) =
            run(conn, id, source, StreamGuard::new(), Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();

        assert_eq!(summary.channels, 3);
        assert_eq!(summary.movies, 2);
        assert_eq!(summary.series, 1);
    }

    #[tokio::test]
    async fn sync_twice_is_idempotent() {
        let conn = memory_conn();
        let id = provider_id(&conn);
        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeSource::with_counts(3, 2, 1));

        let (conn, _) = run(conn, id, Arc::clone(&source), StreamGuard::new(), Arc::new(AtomicBool::new(false)), |_| {})
            .await
            .unwrap();
        let (_conn, summary) =
            run(conn, id, source, StreamGuard::new(), Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();

        assert_eq!(summary.channels, 3);
        assert_eq!(summary.movies, 2);
        assert_eq!(summary.series, 1);
    }

    #[tokio::test]
    async fn sync_fails_when_auth_not_ok() {
        let conn = memory_conn();
        let id = provider_id(&conn);
        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeSource::auth_failed());

        let err = run(conn, id, source, StreamGuard::new(), Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap_err();
        assert!(matches!(err, SyncError::Source(SourceError::AuthFailed)));
    }

    #[tokio::test]
    async fn sync_pauses_while_stream_guard_is_busy_then_finishes() {
        let conn = memory_conn();
        let id = provider_id(&conn);
        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeSource::with_counts(1, 1, 1));
        let guard = StreamGuard::new();
        let cancel = Arc::new(AtomicBool::new(false));

        let permit = guard.acquire().await;
        let guard_for_task = Arc::clone(&guard);
        let handle = tokio::spawn(run(conn, id, source, guard_for_task, cancel, |_| {}));

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!handle.is_finished(), "no debería avanzar mientras StreamGuard está ocupado");

        drop(permit);

        let (_, summary) = tokio::time::timeout(Duration::from_secs(5), handle).await.unwrap().unwrap().unwrap();
        assert_eq!(summary.channels, 1);
    }

    #[tokio::test]
    async fn sync_stops_when_cancelled() {
        let conn = memory_conn();
        let id = provider_id(&conn);
        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeSource::with_counts(1, 0, 0));
        let cancel = Arc::new(AtomicBool::new(true));

        let err = run(conn, id, source, StreamGuard::new(), cancel, |_| {}).await.unwrap_err();
        assert!(matches!(err, SyncError::Cancelled));
    }
}
