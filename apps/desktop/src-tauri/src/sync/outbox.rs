//! Apunta en la cola de salida (`sync_outbox`) los cambios locales que tienen que llegar
//! a la nube. Se llama con la misma conexión que acaba de hacer el cambio: lo que se
//! encola es una foto del estado local en ese momento (existe → documento; ya no existe
//! → borrado lógico), así cualquier secuencia de cambios termina en el último estado.
//!
//! Funciona igual con o sin sesión: lo apuntado sin conexión se envía cuando la haya.
//! Después de encolar, quien llama avisa al motor con `engine::wake()`.

use rand::RngExt;
use rusqlite::Connection;

use super::backend::{RemoteDoc, RemoteFavorite, RemotePlayback, RemoteProfile, RemoteRating};
use crate::db::{cloud, now_epoch, queries, reads, Result};

/// El progreso se guarda cada 10 s mientras se reproduce; a la nube basta con mandarlo
/// como mucho una vez por minuto (la fila pendiente se va sustituyendo mientras tanto).
pub const PROGRESS_DEBOUNCE_S: i64 = 60;

/// Id nuevo para un perfil que nunca se ha subido: 20 caracteres alfanuméricos, mismo
/// formato que los ids automáticos de Firestore. Se decide aquí y no en la nube para
/// poder encolar sin conexión.
pub fn new_remote_id() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..20).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect()
}

/// `remote_id` del perfil, creándolo si aún no tenía. `None` si el perfil no existe.
pub fn ensure_profile_remote_id(conn: &Connection, profile_id: i64) -> Result<Option<String>> {
    let Some(profile) = cloud::sync_profile(conn, profile_id)? else { return Ok(None) };
    if let Some(remote_id) = profile.remote_id {
        return Ok(Some(remote_id));
    }
    let remote_id = new_remote_id();
    queries::set_profile_remote_id(conn, profile_id, &remote_id)?;
    Ok(Some(remote_id))
}

fn put(conn: &Connection, doc: &RemoteDoc, not_before: i64) -> Result<()> {
    let payload = serde_json::to_string(doc)?;
    cloud::enqueue(conn, &doc.target(), doc.collection().as_str(), &payload, not_before)
}

/// Estado actual de un perfil (creado, renombrado, PIN cambiado...).
pub fn enqueue_profile(conn: &Connection, profile_id: i64) -> Result<()> {
    let Some(remote_id) = ensure_profile_remote_id(conn, profile_id)? else { return Ok(()) };
    let Some(profile) = cloud::sync_profile(conn, profile_id)? else { return Ok(()) };
    let doc = RemoteDoc::Profile(RemoteProfile {
        id: remote_id,
        name: profile.name,
        avatar: profile.avatar,
        adult_ok: profile.adult_ok,
        max_age: profile.max_age,
        pin_hash: profile.pin_hash,
        updated_at: profile.updated_at,
        deleted: false,
    });
    put(conn, &doc, now_epoch())
}

/// Perfil borrado en este PC. `remote_id` lo lee quien llama antes de borrar la fila.
/// Sus favoritos y progreso no se borran uno a uno en la nube: los demás dispositivos
/// borran el perfil local entero (con su cascada) al ver este borrado.
pub fn enqueue_profile_deleted(conn: &Connection, remote_id: &str) -> Result<()> {
    cloud::drop_pending_for_profile(conn, remote_id)?;
    let now = now_epoch();
    let doc = RemoteDoc::Profile(RemoteProfile {
        id: remote_id.to_string(),
        name: String::new(),
        avatar: None,
        adult_ok: false,
        max_age: None,
        pin_hash: None,
        updated_at: now,
        deleted: true,
    });
    put(conn, &doc, now)
}

/// Estado actual de un favorito (marcado o quitado). Los de canal no se sincronizan.
pub fn enqueue_favorite(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<()> {
    if item_kind == "channel" {
        return Ok(());
    }
    let Some(item_key) = reads::item_key_for(conn, item_kind, item_id)? else { return Ok(()) };
    let Some(profile_remote_id) = ensure_profile_remote_id(conn, profile_id)? else { return Ok(()) };
    let now = now_epoch();
    let added_at = cloud::favorite_added_at(conn, profile_id, item_kind, item_id)?;
    let doc = RemoteDoc::Favorite(RemoteFavorite {
        profile_id: profile_remote_id,
        item_kind: item_kind.to_string(),
        item_key,
        added_at: added_at.unwrap_or(now),
        updated_at: added_at.unwrap_or(now),
        deleted: added_at.is_none(),
    });
    put(conn, &doc, now)
}

/// Estado actual de una valoración (puesta, cambiada o quitada).
pub fn enqueue_rating(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64) -> Result<()> {
    if item_kind != "movie" && item_kind != "series" {
        return Ok(());
    }
    let Some(item_key) = reads::item_key_for(conn, item_kind, item_id)? else { return Ok(()) };
    let Some(profile_remote_id) = ensure_profile_remote_id(conn, profile_id)? else { return Ok(()) };
    let now = now_epoch();
    let state = cloud::rating_state(conn, profile_id, item_kind, item_id)?;
    let doc = RemoteDoc::Rating(RemoteRating {
        profile_id: profile_remote_id,
        item_kind: item_kind.to_string(),
        item_key,
        rating: state.map_or(0, |(rating, _)| rating),
        updated_at: state.map_or(now, |(_, rated_at)| rated_at),
        deleted: state.is_none(),
    });
    put(conn, &doc, now)
}

/// Estado actual del progreso de un ítem (guardado, marcado como visto o quitado).
/// `debounce` agrupa las escrituras periódicas del reproductor; los cambios hechos a
/// mano (marcar visto, quitar de "Continuar viendo") salen enseguida.
pub fn enqueue_playback(conn: &Connection, profile_id: i64, item_kind: &str, item_id: i64, debounce: bool) -> Result<()> {
    if item_kind == "channel" {
        return Ok(());
    }
    let Some(item_key) = reads::item_key_for(conn, item_kind, item_id)? else { return Ok(()) };
    let Some(profile_remote_id) = ensure_profile_remote_id(conn, profile_id)? else { return Ok(()) };
    let now = now_epoch();
    let doc = match reads::get_playback_progress(conn, profile_id, item_kind, item_id)? {
        Some(p) => RemoteDoc::Playback(RemotePlayback {
            profile_id: profile_remote_id,
            item_kind: item_kind.to_string(),
            item_key,
            position_s: p.position_s,
            duration_s: p.duration_s,
            finished: p.finished,
            played_at: p.played_at,
            updated_at: p.played_at,
            deleted: false,
        }),
        None => RemoteDoc::Playback(RemotePlayback {
            profile_id: profile_remote_id,
            item_kind: item_kind.to_string(),
            item_key,
            position_s: 0,
            duration_s: None,
            finished: false,
            played_at: now,
            updated_at: now,
            deleted: true,
        }),
    };
    put(conn, &doc, if debounce { now + PROGRESS_DEBOUNCE_S } else { now })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_remote_ids_are_twenty_alphanumeric_chars_and_unique() {
        let a = new_remote_id();
        assert_eq!(a.len(), 20);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, new_remote_id());
    }
}
