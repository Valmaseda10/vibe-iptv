//! Motor de sincronización con la nube (F11). Un único bucle en segundo plano que:
//!
//! 1. la primera vez que ve una cuenta en este PC, fusiona lo local con lo de la nube
//!    (sube lo que falte arriba, incluido el progreso, que antes nunca se subía);
//! 2. envía la cola de salida (`sync_outbox`) con reintentos y espera exponencial;
//! 3. cada pocos minutos baja lo que otros dispositivos cambiaron en la nube.
//!
//! Local-first: nada de esto bloquea ni hace fallar una acción del usuario. Sin red o
//! con la nube caída, la cola espera y la app sigue igual. Catálogo, credenciales de
//! proveedor y streaming nunca pasan por aquí. Detalle en `docs/CLOUD-SYNC.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tokio::sync::Notify;

use super::backend::{CloudBackend, CloudError, Collection, RemoteDoc, Session};
use super::{outbox, session};
use crate::db::{cloud, now_epoch, queries, reads, schema};

/// Cada cuánto despierta el bucle aunque nadie lo avise (reintentos, progreso agrupado).
const TICK: Duration = Duration::from_secs(30);
/// Cada cuánto se baja lo cambiado en otros dispositivos.
const PULL_EVERY: Duration = Duration::from_secs(180);
/// Al bajar, se relee este margen antes del cursor: una escritura con marca de servidor
/// anterior a la última vista puede hacerse visible un instante después. Aplicar dos
/// veces el mismo documento no cambia nada.
const PULL_OVERLAP_S: i64 = 120;
/// Documentos por escritura (Firestore admite 500 por `commit`).
const BATCH: i64 = 200;
const MAX_BACKOFF_S: i64 = 3_600;

/// Ajustes de la cuenta sincronizada en `app_settings`; se borran todos al cerrar sesión.
const SETTINGS_PREFIX: &str = "cloud.";
const MERGED_UID_KEY: &str = "cloud.merged_uid";
/// De qué cuenta son los perfiles de este PC. Fuera del prefijo `cloud.`: tiene que
/// sobrevivir al cierre de sesión.
const DATA_OWNER_KEY: &str = "account.data_owner_uid";

fn cursor_key(collection: Collection) -> String {
    format!("cloud.cursor.{}", collection.as_str())
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[error("error local de sincronización: {0}")]
    Local(String),
}

/// Cómo fue la última vuelta del motor, para enseñarlo en Ajustes.
#[derive(Debug, Clone, Default)]
pub struct EngineStatus {
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
}

struct Engine {
    db_path: PathBuf,
    cloud: Arc<dyn CloudBackend>,
    wake: Notify,
    status: Mutex<EngineStatus>,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

/// Arranca el bucle en segundo plano. Solo la primera llamada tiene efecto.
pub fn start(db_path: PathBuf, cloud: Arc<dyn CloudBackend>) {
    session::clear_legacy();
    let engine = Arc::new(Engine { db_path, cloud, wake: Notify::new(), status: Mutex::new(EngineStatus::default()) });
    if ENGINE.set(engine.clone()).is_ok() {
        tokio::spawn(engine.run());
    }
}

/// Avisa al motor de que hay algo nuevo (un cambio encolado, una sesión recién
/// iniciada) para que no espere al siguiente ciclo. Sin motor arrancado (tests), nada.
pub fn wake() {
    if let Some(engine) = ENGINE.get() {
        engine.wake.notify_one();
    }
}

pub fn status() -> EngineStatus {
    ENGINE.get().and_then(|e| e.status.lock().ok().map(|s| s.clone())).unwrap_or_default()
}

impl Engine {
    async fn run(self: Arc<Self>) {
        let mut last_pull: Option<Instant> = None;
        loop {
            if let Some(result) = self.tick(&mut last_pull).await {
                if let Ok(mut status) = self.status.lock() {
                    match result {
                        Ok(()) => *status = EngineStatus { last_synced_at: Some(now_epoch()), last_error: None },
                        Err(err) => {
                            tracing::info!(error = %err, "la sincronización con la nube se reintentará");
                            status.last_error = Some(err.to_string());
                        }
                    }
                }
            }
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(TICK) => {}
            }
        }
    }

    /// `None` si no hay sesión: lo encolado espera a que el usuario inicie sesión.
    async fn tick(&self, last_pull: &mut Option<Instant>) -> Option<Result<(), SyncError>> {
        let session = session::get_valid(self.cloud.as_ref()).await?;
        // Recién arrancada la app no hay token de acceso en memoria; si renovarlo falló
        // por algo pasajero, no tiene sentido mandar peticiones con un token vacío.
        if session.access_token.is_empty() {
            return Some(Err(CloudError::Unavailable.into()));
        }
        let cloud = self.cloud.as_ref();
        Some(
            async {
                initial_merge_if_needed(&self.db_path, cloud, &session).await?;
                flush_outbox(&self.db_path, cloud, &session).await?;
                if last_pull.is_none_or(|t| t.elapsed() >= PULL_EVERY) {
                    pull_changes(&self.db_path, cloud, &session).await?;
                    *last_pull = Some(Instant::now());
                }
                Ok(())
            }
            .await,
        )
    }
}

async fn with_db<T, F>(db_path: &Path, f: F) -> Result<T, SyncError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> crate::db::Result<T> + Send + 'static,
{
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let conn = schema::open_connection_at(&db_path)?;
        f(&conn)
    })
    .await
    .map_err(|err| SyncError::Local(err.to_string()))?
    .map_err(|err| SyncError::Local(err.to_string()))
}

/// Cambios locales todavía sin llegar a la nube.
pub async fn pending_changes(db_path: &Path) -> i64 {
    with_db(db_path, cloud::pending_count).await.unwrap_or(0)
}

/// Al cerrar sesión: lo pendiente y los cursores eran de esa cuenta, no de la próxima.
/// Los perfiles se quedan (si vuelve la misma cuenta, nada se pierde), pero apuntando de
/// quién son: ver [`claim_local_data`].
pub async fn forget_account(db_path: &Path) -> Result<(), SyncError> {
    with_db(db_path, |conn| {
        if let Some(owner) = reads::get_setting(conn, MERGED_UID_KEY)? {
            queries::set_setting(conn, DATA_OWNER_KEY, &owner)?;
        }
        cloud::clear_outbox(conn)?;
        cloud::delete_settings_with_prefix(conn, SETTINGS_PREFIX)
    })
    .await
}

/// Antes de usar una cuenta en este PC. Si los perfiles locales son de otra cuenta, se
/// quitan, con sus favoritos, progreso y cambios pendientes: si no, la fusión los subiría
/// a esta como si fueran suyos. Si no son de nadie (creados antes de iniciar sesión
/// nunca), pasan a ser de esta.
pub async fn claim_local_data(db_path: &Path, user_id: &str) -> Result<(), SyncError> {
    let user_id = user_id.to_string();
    with_db(db_path, move |conn| {
        let owner = match reads::get_setting(conn, DATA_OWNER_KEY)? {
            Some(owner) => Some(owner),
            None => reads::get_setting(conn, MERGED_UID_KEY)?,
        };
        let tx = conn.unchecked_transaction()?;
        if owner.as_deref().is_some_and(|owner| owner != user_id) {
            cloud::delete_account_data(&tx)?;
            cloud::delete_settings_with_prefix(&tx, SETTINGS_PREFIX)?;
            tracing::info!("los perfiles de este PC eran de otra cuenta: se han quitado");
        }
        queries::set_setting(&tx, DATA_OWNER_KEY, &user_id)?;
        tx.commit()?;
        Ok(())
    })
    .await
}

/// Espera antes de reintentar una entrada que ya falló `attempts` veces: 30 s, 1 min,
/// 2 min... hasta una hora como mucho.
fn backoff_seconds(attempts: i64) -> i64 {
    (30_i64 << attempts.clamp(0, 10)).min(MAX_BACKOFF_S)
}

/// Envía todo lo que ya toca de la cola de salida.
pub async fn flush_outbox(db_path: &Path, cloud: &dyn CloudBackend, session: &Session) -> Result<usize, SyncError> {
    flush_outbox_at(db_path, cloud, session, now_epoch()).await
}

async fn flush_outbox_at(db_path: &Path, cloud: &dyn CloudBackend, session: &Session, now: i64) -> Result<usize, SyncError> {
    let mut sent = 0;
    loop {
        let entries = with_db(db_path, move |conn| cloud::due_entries(conn, now, BATCH)).await?;
        if entries.is_empty() {
            break;
        }
        let batch_was_full = entries.len() as i64 == BATCH;

        let mut docs = Vec::with_capacity(entries.len());
        let mut sending = Vec::with_capacity(entries.len());
        let mut unreadable = Vec::new();
        for entry in entries {
            match serde_json::from_str::<RemoteDoc>(&entry.payload) {
                Ok(doc) => {
                    docs.push(doc);
                    sending.push(entry);
                }
                Err(_) => {
                    // Nunca debería pasar (lo escribe `sync::outbox`); si pasa, una entrada
                    // ilegible no puede bloquear para siempre a las demás.
                    tracing::warn!(target = %entry.target, "entrada ilegible en la cola de sincronización; se descarta");
                    unreadable.push((entry.id, entry.revision));
                }
            }
        }
        if !unreadable.is_empty() {
            with_db(db_path, move |conn| unreadable.iter().try_for_each(|(id, revision)| cloud::ack(conn, *id, *revision))).await?;
        }

        match cloud.write(session, &docs).await {
            Ok(()) => {
                sent += docs.len();
                with_db(db_path, move |conn| sending.iter().try_for_each(|e| cloud::ack(conn, e.id, e.revision))).await?;
            }
            Err(err) => {
                let message = err.to_string();
                with_db(db_path, move |conn| {
                    sending.iter().try_for_each(|e| cloud::record_failure(conn, e.id, e.revision, &message, now + backoff_seconds(e.attempts)))
                })
                .await?;
                return Err(err.into());
            }
        }
        if !batch_was_full {
            break;
        }
    }
    Ok(sent)
}

/// Baja y aplica lo cambiado en la nube desde la última vez, colección a colección
/// (perfiles primero: favoritos y progreso cuelgan de ellos).
pub async fn pull_changes(db_path: &Path, cloud: &dyn CloudBackend, session: &Session) -> Result<(), SyncError> {
    for collection in Collection::ALL {
        let key = cursor_key(collection);
        let cursor = with_db(db_path, {
            let key = key.clone();
            move |conn| reads::get_setting(conn, &key)
        })
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

        let changes = cloud.changes_since(session, collection, (cursor - PULL_OVERLAP_S).max(0)).await?;
        let new_cursor = changes.max_server_ts.map_or(cursor, |ts| ts.max(cursor));
        let docs = changes.docs;
        with_db(db_path, move |conn| {
            let tx = conn.unchecked_transaction()?;
            for doc in &docs {
                apply_remote(&tx, doc)?;
            }
            queries::set_setting(&tx, &key, &new_cursor.to_string())?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// La primera vez que una cuenta se usa en este PC: baja todo lo que hay en la nube, lo
/// aplica, y encola lo local que la nube no tenga (o tenga más viejo). Cubre también la
/// migración desde Supabase: la nube nueva empieza vacía y recibe todo lo de este PC.
pub async fn initial_merge_if_needed(db_path: &Path, cloud: &dyn CloudBackend, session: &Session) -> Result<(), SyncError> {
    let merged_uid = with_db(db_path, |conn| reads::get_setting(conn, MERGED_UID_KEY)).await?;
    if merged_uid.as_deref() == Some(session.user_id.as_str()) {
        return Ok(());
    }
    // El inicio de sesión ya lo hace; esto cubre una sesión que llegue por otro camino.
    claim_local_data(db_path, &session.user_id).await?;

    let mut remote = Vec::new();
    let mut cursors = Vec::new();
    for collection in Collection::ALL {
        let changes = cloud.changes_since(session, collection, 0).await?;
        cursors.push((cursor_key(collection), changes.max_server_ts.unwrap_or(0)));
        remote.extend(changes.docs);
    }

    let user_id = session.user_id.clone();
    with_db(db_path, move |conn| {
        let tx = conn.unchecked_transaction()?;
        for doc in &remote {
            apply_remote(&tx, doc)?;
        }
        enqueue_local_missing_from_cloud(&tx, &remote)?;
        for (key, ts) in &cursors {
            queries::set_setting(&tx, key, &ts.to_string())?;
        }
        queries::set_setting(&tx, MERGED_UID_KEY, &user_id)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    tracing::info!("primera fusión con la nube completada");
    Ok(())
}

/// Tras aplicar lo de la nube, todo lo local que siga sin pareja arriba (o con una más
/// vieja) se encola. Si un borrado remoto sobrevivió a `apply_remote`, es que lo local es
/// más nuevo, así que también se sube.
fn enqueue_local_missing_from_cloud(conn: &Connection, remote: &[RemoteDoc]) -> crate::db::Result<()> {
    let by_target: HashMap<String, &RemoteDoc> = remote.iter().map(|doc| (doc.target(), doc)).collect();

    for profile in cloud::sync_profiles(conn)? {
        let Some(remote_id) = outbox::ensure_profile_remote_id(conn, profile.id)? else { continue };
        let needs_upload = match by_target.get(&RemoteDoc::profile_target(&remote_id)) {
            Some(RemoteDoc::Profile(r)) => r.deleted || r.updated_at < profile.updated_at,
            _ => true,
        };
        if needs_upload {
            outbox::enqueue_profile(conn, profile.id)?;
        }

        for favorite in cloud::sync_favorites(conn, profile.id)? {
            let Some(item_key) = reads::item_key_for(conn, &favorite.item_kind, favorite.item_id)? else { continue };
            let needs_upload = match by_target.get(&RemoteDoc::favorite_target(&remote_id, &favorite.item_kind, &item_key)) {
                Some(RemoteDoc::Favorite(r)) => r.deleted,
                _ => true,
            };
            if needs_upload {
                outbox::enqueue_favorite(conn, profile.id, &favorite.item_kind, favorite.item_id)?;
            }
        }

        for progress in cloud::sync_playback(conn, profile.id)? {
            let Some(item_key) = reads::item_key_for(conn, &progress.item_kind, progress.item_id)? else { continue };
            let needs_upload = match by_target.get(&RemoteDoc::playback_target(&remote_id, &progress.item_kind, &item_key)) {
                Some(RemoteDoc::Playback(r)) => r.deleted || r.played_at < progress.played_at,
                _ => true,
            };
            if needs_upload {
                outbox::enqueue_playback(conn, profile.id, &progress.item_kind, progress.item_id, false)?;
            }
        }
    }
    Ok(())
}

/// Aplica un documento de la nube sobre la base local. Gana la escritura más reciente
/// (marca del cliente que la hizo), y un cambio local todavía en la cola gana siempre:
/// se enviará y sobrescribirá lo de arriba. Lo que el catálogo de este PC no tiene
/// (otro proveedor) se ignora.
fn apply_remote(conn: &Connection, doc: &RemoteDoc) -> crate::db::Result<()> {
    if cloud::has_pending_target(conn, &doc.target())? {
        return Ok(());
    }
    match doc {
        RemoteDoc::Profile(r) => {
            let local = cloud::profile_by_remote_id(conn, &r.id)?;
            if r.deleted {
                // Nunca se borra el último perfil: la app siempre necesita al menos uno.
                if let Some(local) = local {
                    if r.updated_at >= local.updated_at && reads::count_profiles(conn)? > 1 {
                        queries::delete_profile(conn, local.id)?;
                    }
                }
            } else if local.is_none_or(|l| r.updated_at > l.updated_at) {
                cloud::upsert_profile_from_remote(conn, &r.id, &r.name, r.avatar.as_deref(), r.adult_ok, r.max_age, r.pin_hash.as_deref(), r.updated_at)?;
            }
        }
        RemoteDoc::Favorite(f) => {
            let Some(profile) = cloud::profile_by_remote_id(conn, &f.profile_id)? else { return Ok(()) };
            let Some(item_id) = reads::item_id_for_key(conn, &f.item_kind, &f.item_key)? else { return Ok(()) };
            let local_added_at = cloud::favorite_added_at(conn, profile.id, &f.item_kind, item_id)?;
            if f.deleted {
                if local_added_at.is_some_and(|added_at| f.updated_at >= added_at) {
                    reads::remove_favorite(conn, profile.id, &f.item_kind, item_id)?;
                }
            } else if local_added_at.is_none() {
                cloud::put_favorite_from_remote(conn, profile.id, &f.item_kind, item_id, f.added_at)?;
            }
        }
        RemoteDoc::Playback(p) => {
            let Some(profile) = cloud::profile_by_remote_id(conn, &p.profile_id)? else { return Ok(()) };
            let Some(item_id) = reads::item_id_for_key(conn, &p.item_kind, &p.item_key)? else { return Ok(()) };
            let local = reads::get_playback_progress(conn, profile.id, &p.item_kind, item_id)?;
            if p.deleted {
                if local.is_some_and(|l| p.updated_at >= l.played_at) {
                    reads::remove_playback_progress(conn, profile.id, &p.item_kind, item_id)?;
                }
            } else if local.is_none_or(|l| p.played_at > l.played_at) {
                cloud::put_playback_from_remote(conn, profile.id, &p.item_kind, item_id, p.position_s, p.duration_s, p.finished, p.played_at)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::backend::{RemoteFavorite, RemotePlayback};
    use crate::sync::memory::MemoryBackend;

    /// Un "dispositivo": su propia base SQLite con el mismo catálogo mínimo (una
    /// película y un episodio con `match_key` estables) y un perfil.
    struct Device {
        db: PathBuf,
        profile_id: i64,
    }

    impl Device {
        fn new(label: &str) -> Self {
            use rand::RngExt;
            let suffix: u64 = rand::rng().random();
            let db = std::env::temp_dir().join(format!("vibe-iptv-engine-test-{label}-{}-{suffix:x}.sqlite3", std::process::id()));
            let conn = schema::open_connection_at(&db).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, kind, base_url, created_at) VALUES (1, 'P', 'xtream', 'http://x', 0);
                 INSERT INTO movies (id, clean_title, match_key) VALUES (10, 'Matrix', 'matrix-1999');
                 INSERT INTO series (id, clean_title, match_key) VALUES (20, 'Serie', 'serie-2020');
                 INSERT INTO series_sources (id, series_id, provider_id, remote_id, raw_title) VALUES (30, 20, 1, 's1', 'Serie');
                 INSERT INTO episodes (id, series_source_id, remote_id, season, number) VALUES (40, 30, 'e1', 1, 2);",
            )
            .unwrap();
            let profile_id = queries::insert_profile(&conn, "Ana", None, true).unwrap();
            Self { db, profile_id }
        }

        fn conn(&self) -> Connection {
            schema::open_connection_at(&self.db).unwrap()
        }

        fn favorite_movie(&self) {
            let conn = self.conn();
            reads::add_favorite(&conn, self.profile_id, "movie", 10).unwrap();
            outbox::enqueue_favorite(&conn, self.profile_id, "movie", 10).unwrap();
        }

        fn unfavorite_movie(&self) {
            let conn = self.conn();
            reads::remove_favorite(&conn, self.profile_id, "movie", 10).unwrap();
            outbox::enqueue_favorite(&conn, self.profile_id, "movie", 10).unwrap();
        }

        fn watch_episode(&self, position_s: i64) {
            let conn = self.conn();
            reads::upsert_playback_progress(&conn, self.profile_id, "episode", 40, position_s, Some(1_800)).unwrap();
            outbox::enqueue_playback(&conn, self.profile_id, "episode", 40, true).unwrap();
        }

        /// En cualquier perfil: tras bajar, el favorito cuelga del perfil espejo del otro
        /// dispositivo, no del propio.
        fn has_favorite_movie(&self) -> bool {
            self.conn()
                .query_row("SELECT EXISTS(SELECT 1 FROM favorites WHERE item_kind = 'movie' AND item_id = 10)", [], |r| r.get(0))
                .unwrap()
        }

        fn episode_progress(&self) -> Option<reads::PlaybackItem> {
            let conn = self.conn();
            let profile_id: Option<i64> = conn
                .query_row("SELECT profile_id FROM playback_state WHERE item_kind = 'episode' AND item_id = 40", [], |r| r.get(0))
                .ok();
            reads::get_playback_progress(&conn, profile_id?, "episode", 40).unwrap()
        }

        fn pending(&self) -> i64 {
            cloud::pending_count(&self.conn()).unwrap()
        }
    }

    /// Envía lo pendiente como si ya hubieran pasado `hours` horas: más que la agrupación
    /// del progreso y que cualquier espera tras un fallo.
    async fn flush_after_hours(device: &Device, cloud: &MemoryBackend, hours: i64) -> Result<usize, SyncError> {
        flush_outbox_at(&device.db, cloud, &cloud.session(), now_epoch() + hours * 3_600).await
    }

    async fn flush_everything(device: &Device, cloud: &MemoryBackend) -> Result<usize, SyncError> {
        flush_after_hours(device, cloud, 2).await
    }

    fn local_profiles(device: &Device) -> i64 {
        device.conn().query_row("SELECT COUNT(*) FROM profiles", [], |r| r.get(0)).unwrap()
    }

    /// Lo que pasó en la app real: se cerró sesión en una cuenta, se entró con otra y la
    /// fusión subió a la nueva los perfiles de la anterior.
    #[tokio::test]
    async fn another_account_never_inherits_the_profiles_left_on_this_pc() {
        let cloud_a = MemoryBackend::new();
        let cloud_b = MemoryBackend::new();
        let session_a = cloud_a.session();
        let session_b = Session { user_id: "uid-otra-cuenta".into(), ..cloud_b.session() };
        let pc = Device::new("cuentas");
        pc.favorite_movie();

        claim_local_data(&pc.db, &session_a.user_id).await.unwrap();
        initial_merge_if_needed(&pc.db, &cloud_a, &session_a).await.unwrap();
        flush_outbox_at(&pc.db, &cloud_a, &session_a, now_epoch() + 7_200).await.unwrap();
        assert_eq!(cloud_a.docs(Collection::Profiles).len(), 1);

        // Cerrar sesión y volver con la misma cuenta no pierde nada.
        forget_account(&pc.db).await.unwrap();
        claim_local_data(&pc.db, &session_a.user_id).await.unwrap();
        assert_eq!(local_profiles(&pc), 1);
        assert!(pc.has_favorite_movie());

        // Con otra cuenta, los perfiles de la anterior desaparecen del PC y no suben.
        forget_account(&pc.db).await.unwrap();
        claim_local_data(&pc.db, &session_b.user_id).await.unwrap();
        assert_eq!(local_profiles(&pc), 0);
        assert!(!pc.has_favorite_movie());
        initial_merge_if_needed(&pc.db, &cloud_b, &session_b).await.unwrap();
        flush_outbox_at(&pc.db, &cloud_b, &session_b, now_epoch() + 7_200).await.unwrap();
        assert!(cloud_b.docs(Collection::Profiles).is_empty(), "la cuenta nueva recibió perfiles ajenos");
        assert!(cloud_b.docs(Collection::Favorites).is_empty());
    }

    /// Sin cerrar sesión (la sesión caducó y se entra con otra cuenta), la fusión tampoco
    /// sube lo de la cuenta anterior, ni lo que esta tenía pendiente.
    #[tokio::test]
    async fn an_expired_session_followed_by_another_account_does_not_leak_either() {
        let cloud_a = MemoryBackend::new();
        let cloud_b = MemoryBackend::new();
        let session_a = cloud_a.session();
        let session_b = Session { user_id: "uid-otra-cuenta".into(), ..cloud_b.session() };
        let pc = Device::new("caducada");
        initial_merge_if_needed(&pc.db, &cloud_a, &session_a).await.unwrap();
        pc.favorite_movie();
        assert!(pc.pending() > 0);

        initial_merge_if_needed(&pc.db, &cloud_b, &session_b).await.unwrap();
        flush_outbox_at(&pc.db, &cloud_b, &session_b, now_epoch() + 7_200).await.unwrap();
        assert_eq!(local_profiles(&pc), 0);
        assert!(cloud_b.docs(Collection::Profiles).is_empty());
        assert!(cloud_b.docs(Collection::Favorites).is_empty(), "el favorito pendiente de la otra cuenta llegó a esta");
    }

    #[tokio::test]
    async fn changes_made_while_the_cloud_is_down_are_delivered_when_it_returns() {
        let cloud = MemoryBackend::new();
        let pc = Device::new("offline");
        pc.favorite_movie();
        pc.watch_episode(300);

        cloud.fail_everything(Some(CloudError::Unavailable));
        assert!(flush_after_hours(&pc, &cloud, 2).await.is_err());
        assert_eq!(pc.pending(), 2, "nada se pierde mientras la nube está caída");
        assert_eq!(flush_after_hours(&pc, &cloud, 2).await.unwrap(), 0, "tras un fallo se espera antes de reintentar");

        cloud.fail_everything(None);
        assert_eq!(flush_after_hours(&pc, &cloud, 4).await.unwrap(), 2);
        assert_eq!(pc.pending(), 0);
        assert_eq!(cloud.docs(Collection::Favorites).len(), 1);
        let [RemoteDoc::Playback(progress)] = &cloud.docs(Collection::Playback)[..] else { panic!("se esperaba un progreso") };
        assert_eq!(progress.position_s, 300);
        assert_eq!(progress.item_key, "serie-2020:1:2");
    }

    #[tokio::test]
    async fn frequent_progress_saves_collapse_into_one_write() {
        let cloud = MemoryBackend::new();
        let pc = Device::new("debounce");
        for position in [10, 20, 30, 40] {
            pc.watch_episode(position);
        }
        assert_eq!(pc.pending(), 1);
        assert_eq!(flush_outbox(&pc.db, &cloud, &cloud.session()).await.unwrap(), 0, "aún dentro del minuto de agrupación");

        flush_everything(&pc, &cloud).await.unwrap();
        assert_eq!(cloud.write_calls(), 1);
        let [RemoteDoc::Playback(progress)] = &cloud.docs(Collection::Playback)[..] else { panic!("se esperaba un progreso") };
        assert_eq!(progress.position_s, 40, "llega el último estado");
    }

    #[tokio::test]
    async fn initial_merge_uploads_local_profiles_favorites_and_progress() {
        let cloud = MemoryBackend::new();
        let session = cloud.session();
        let pc = Device::new("merge");
        // Estado creado antes de existir la cola (o antes de iniciar sesión): no hay nada
        // encolado, solo filas locales.
        {
            let conn = pc.conn();
            reads::add_favorite(&conn, pc.profile_id, "movie", 10).unwrap();
            reads::upsert_playback_progress(&conn, pc.profile_id, "movie", 10, 120, Some(7_200)).unwrap();
        }
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();

        assert_eq!(cloud.docs(Collection::Profiles).len(), 1);
        assert_eq!(cloud.docs(Collection::Favorites).len(), 1);
        assert_eq!(cloud.docs(Collection::Playback).len(), 1, "el progreso local también sube (antes no)");

        // Una segunda vez con la misma cuenta no repite la fusión.
        let writes = cloud.write_calls();
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();
        assert_eq!(cloud.write_calls(), writes);
    }

    #[tokio::test]
    async fn another_device_sees_additions_and_removals_without_signing_in_again() {
        let cloud = MemoryBackend::new();
        let session = cloud.session();
        let pc = Device::new("pc");
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();

        // El segundo dispositivo entra con la misma cuenta y baja el perfil del PC.
        let laptop = Device::new("laptop");
        initial_merge_if_needed(&laptop.db, &cloud, &session).await.unwrap();
        flush_everything(&laptop, &cloud).await.unwrap();

        pc.favorite_movie();
        pc.watch_episode(900);
        flush_everything(&pc, &cloud).await.unwrap();
        pull_changes(&laptop.db, &cloud, &session).await.unwrap();
        assert!(laptop.has_favorite_movie());
        assert_eq!(laptop.episode_progress().unwrap().position_s, 900);

        pc.unfavorite_movie();
        flush_everything(&pc, &cloud).await.unwrap();
        pull_changes(&laptop.db, &cloud, &session).await.unwrap();
        assert!(!laptop.has_favorite_movie(), "el borrado lógico llega al otro dispositivo");
    }

    #[tokio::test]
    async fn pulled_progress_keeps_its_original_date() {
        let cloud = MemoryBackend::new();
        let session = cloud.session();
        let pc = Device::new("dates");
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();
        let profile_remote_id = cloud::sync_profiles(&pc.conn()).unwrap()[0].remote_id.clone().unwrap();

        cloud.put_from_other_device(RemoteDoc::Playback(RemotePlayback {
            profile_id: profile_remote_id,
            item_kind: "episode".into(),
            item_key: "serie-2020:1:2".into(),
            position_s: 50,
            duration_s: Some(1_800),
            finished: false,
            played_at: 1_000,
            updated_at: 1_000,
            deleted: false,
        }));
        pull_changes(&pc.db, &cloud, &session).await.unwrap();
        let progress = pc.episode_progress().unwrap();
        assert_eq!(progress.played_at, 1_000, "visto hace tiempo en otro sitio no salta al principio de Continuar viendo");
        assert_eq!(pc.pending(), 0, "aplicar lo bajado no genera subidas");
    }

    #[tokio::test]
    async fn a_pending_local_change_wins_over_the_cloud() {
        let cloud = MemoryBackend::new();
        let session = cloud.session();
        let pc = Device::new("pending");
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();
        let profile_remote_id = cloud::sync_profiles(&pc.conn()).unwrap()[0].remote_id.clone().unwrap();

        // Marcado sin conexión aquí, y a la vez quitado desde otro dispositivo.
        pc.favorite_movie();
        cloud.put_from_other_device(RemoteDoc::Favorite(RemoteFavorite {
            profile_id: profile_remote_id,
            item_kind: "movie".into(),
            item_key: "matrix-1999".into(),
            added_at: now_epoch() + 3_600,
            updated_at: now_epoch() + 3_600,
            deleted: true,
        }));
        pull_changes(&pc.db, &cloud, &session).await.unwrap();
        assert!(pc.has_favorite_movie(), "lo encolado aquí todavía no se ha enviado: no se pisa");
    }

    #[tokio::test]
    async fn a_profile_deleted_elsewhere_is_deleted_here_but_never_the_last_one() {
        let cloud = MemoryBackend::new();
        let session = cloud.session();
        let pc = Device::new("profiles");
        let second = queries::insert_profile(&pc.conn(), "Niños", None, false).unwrap();
        initial_merge_if_needed(&pc.db, &cloud, &session).await.unwrap();
        flush_everything(&pc, &cloud).await.unwrap();
        assert_eq!(cloud.docs(Collection::Profiles).len(), 2);

        let remote_ids: Vec<String> = cloud::sync_profiles(&pc.conn()).unwrap().into_iter().filter_map(|p| p.remote_id).collect();
        for remote_id in &remote_ids {
            outbox::enqueue_profile_deleted(&pc.conn(), remote_id).unwrap();
            let doc = serde_json::from_str::<RemoteDoc>(&cloud::due_entries(&pc.conn(), i64::MAX, 10).unwrap()[0].payload).unwrap();
            cloud::clear_outbox(&pc.conn()).unwrap();
            cloud.put_from_other_device(doc);
        }
        pull_changes(&pc.db, &cloud, &session).await.unwrap();
        let left = cloud::sync_profiles(&pc.conn()).unwrap();
        assert_eq!(left.len(), 1, "se borra uno, pero el último se queda");
        assert!(left[0].id == pc.profile_id || left[0].id == second);
    }

    #[test]
    fn backoff_grows_and_is_capped_at_an_hour() {
        assert_eq!(backoff_seconds(0), 30);
        assert_eq!(backoff_seconds(1), 60);
        assert_eq!(backoff_seconds(3), 240);
        assert_eq!(backoff_seconds(10), MAX_BACKOFF_S);
        assert_eq!(backoff_seconds(1_000), MAX_BACKOFF_S);
    }
}
