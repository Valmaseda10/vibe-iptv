//! La clave de TMDB viaja con la cuenta (2026-10-31, pedido por el usuario): se pone en un
//! aparato y llega a los demás; si se cambia en cualquiera, gana la más reciente.
//!
//! En cada aparato vive en el llavero del sistema, como siempre. La cola de salida
//! (SQLite) nunca la guarda: apunta solo que hay que subirla, y el valor se lee del
//! llavero justo al enviarlo (regla R5: nada secreto en la base de datos). En la nube va
//! en un documento de la cuenta (`users/{uid}/settings`), que solo ve su dueño. Ver
//! `docs/CLOUD-SYNC.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use rusqlite::Connection;

use super::backend::{RemoteDoc, RemoteSetting};
use crate::db::{now_epoch, queries, reads};

/// Id del ajuste en la nube.
pub const SETTING_KEY: &str = "tmdb_api_key";
/// Cuándo se puso la clave que tiene este aparato (epoch), en `app_settings`.
const UPDATED_AT_KEY: &str = "tmdb_api_key_updated_at";

static ARRIVED: AtomicBool = AtomicBool::new(false);
static ON_ARRIVED: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static LEGACY_CHECKED: AtomicBool = AtomicBool::new(false);

/// Qué hacer cuando llega una clave nueva de otro aparato (empezar a emparejar con TMDB).
pub fn on_arrived(callback: impl Fn() + Send + Sync + 'static) {
    let _ = ON_ARRIVED.set(Box::new(callback));
}

/// Tras aplicar lo bajado de la nube: si llegó una clave nueva, avisa.
pub fn notify_if_arrived() {
    if ARRIVED.swap(false, Ordering::SeqCst) {
        if let Some(callback) = ON_ARRIVED.get() {
            callback();
        }
    }
}

/// La clave se acaba de poner en este aparato: se apunta cuándo y se encola su subida.
pub fn mark_changed(conn: &Connection) -> crate::db::Result<()> {
    let now = now_epoch();
    queries::set_setting(conn, UPDATED_AT_KEY, &now.to_string())?;
    super::outbox::enqueue_setting(conn, SETTING_KEY, now)
}

/// Cuándo se puso la clave de este aparato, si se sabe.
pub fn local_updated_at(conn: &Connection) -> crate::db::Result<Option<i64>> {
    Ok(reads::get_setting(conn, UPDATED_AT_KEY)?.and_then(|v| v.parse().ok()))
}

/// Una clave guardada antes de que existiera esto no tiene fecha: se le pone la de ahora y
/// se sube. Una vez por arranque.
pub fn stamp_legacy(conn: &Connection) -> crate::db::Result<()> {
    if LEGACY_CHECKED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    if local_updated_at(conn)?.is_none() && read_local().is_some() {
        mark_changed(conn)?;
    }
    Ok(())
}

/// Justo antes de enviar: el documento encolado va sin valor y se rellena desde el
/// llavero. `None` si ya no hay clave (no se envía nada).
pub fn fill_value(doc: RemoteDoc) -> Option<RemoteDoc> {
    match doc {
        RemoteDoc::Setting(mut s) if s.key == SETTING_KEY => {
            s.value = read_local()?;
            Some(RemoteDoc::Setting(s))
        }
        other => Some(other),
    }
}

/// Aplica la clave bajada de la nube si es más reciente que la de este aparato. Devuelve
/// si cambió.
pub fn apply_remote(conn: &Connection, setting: &RemoteSetting) -> crate::db::Result<bool> {
    if setting.key != SETTING_KEY || setting.deleted || setting.value.trim().is_empty() {
        return Ok(false);
    }
    let local_at = local_updated_at(conn)?.unwrap_or(0);
    if setting.updated_at <= local_at {
        return Ok(false);
    }
    if read_local().as_deref() != Some(setting.value.as_str()) {
        if let Err(err) = store_local(&setting.value) {
            tracing::warn!(error = %err, "no se pudo guardar en el llavero la clave de TMDB de la cuenta");
            return Ok(false);
        }
        ARRIVED.store(true, Ordering::SeqCst);
    }
    queries::set_setting(conn, UPDATED_AT_KEY, &setting.updated_at.to_string())?;
    Ok(true)
}

/// Si la clave de este aparato es más nueva que la de la nube (o la nube no tiene), hay que
/// subirla. Para la primera fusión de una cuenta.
pub fn needs_upload(conn: &Connection, remote: Option<&RemoteSetting>) -> crate::db::Result<bool> {
    let Some(local_at) = local_updated_at(conn)? else { return Ok(false) };
    if read_local().is_none() {
        return Ok(false);
    }
    Ok(remote.is_none_or(|r| r.deleted || r.updated_at < local_at))
}

#[cfg(not(test))]
fn read_local() -> Option<String> {
    crate::util::keychain::read_secret(crate::catalog::tmdb::TMDB_API_KEY_CREDENTIAL_REF).ok().filter(|k| !k.trim().is_empty())
}

#[cfg(not(test))]
fn store_local(value: &str) -> Result<(), String> {
    crate::util::keychain::store_secret(crate::catalog::tmdb::TMDB_API_KEY_CREDENTIAL_REF, value).map_err(|err| err.to_string())
}

// En las pruebas, un llavero de mentira: el de verdad es el del sistema.
#[cfg(test)]
pub(crate) static TEST_KEYCHAIN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn read_local() -> Option<String> {
    TEST_KEYCHAIN.lock().ok()?.clone()
}

#[cfg(test)]
fn store_local(value: &str) -> Result<(), String> {
    *TEST_KEYCHAIN.lock().map_err(|_| "llavero de pruebas")? = Some(value.to_string());
    Ok(())
}
