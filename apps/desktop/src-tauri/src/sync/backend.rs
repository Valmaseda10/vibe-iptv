//! Contrato entre el motor de sincronización y el servicio en la nube que guarda la
//! cuenta, los perfiles, los favoritos y el progreso. Un trait, igual que
//! `sources::Source` aísla al proveedor IPTV: `sync::engine` no sabe nada de Firebase,
//! y sus tests lo ejercitan contra un backend en memoria, sin red.
//!
//! El modelo de documentos y el porqué de cada campo están en `docs/CLOUD-SYNC.md`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
    #[error("No se pudo conectar con el servidor de cuentas. Comprueba tu conexión a internet.")]
    Network,
    #[error("El servidor de cuentas no está disponible ahora mismo. Vuelve a intentarlo en unos minutos.")]
    Unavailable,
    #[error("Tu sesión ha caducado. Vuelve a iniciar sesión.")]
    SessionExpired,
    /// Rechazo con un motivo concreto, ya redactado para enseñarlo al usuario.
    #[error("{0}")]
    Rejected(String),
    #[error("Respuesta inesperada del servidor de cuentas.")]
    InvalidResponse,
}

impl CloudError {
    /// Fallos que no dicen nada de la cuenta del usuario: sin red, servidor caído o una
    /// respuesta que no se entiende. Ninguno debe cerrar la sesión ni bloquear la app.
    pub fn is_transient(&self) -> bool {
        matches!(self, CloudError::Network | CloudError::Unavailable | CloudError::InvalidResponse)
    }
}

pub type Result<T> = std::result::Result<T, CloudError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub email: String,
    /// Epoch en el que caduca `access_token`; `sync::session` refresca antes.
    pub expires_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collection {
    Profiles,
    Favorites,
    Playback,
    Ratings,
}

impl Collection {
    pub const ALL: [Collection; 4] = [Collection::Profiles, Collection::Favorites, Collection::Playback, Collection::Ratings];

    pub fn as_str(self) -> &'static str {
        match self {
            Collection::Profiles => "profiles",
            Collection::Favorites => "favorites",
            Collection::Playback => "playback",
            Collection::Ratings => "ratings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteProfile {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub adult_ok: bool,
    /// Edad máxima (F14); `None` es sin límite.
    #[serde(default)]
    pub max_age: Option<i64>,
    pub pin_hash: Option<String>,
    pub updated_at: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteFavorite {
    pub profile_id: String,
    pub item_kind: String,
    pub item_key: String,
    pub added_at: i64,
    pub updated_at: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemotePlayback {
    pub profile_id: String,
    pub item_kind: String,
    pub item_key: String,
    pub position_s: i64,
    pub duration_s: Option<i64>,
    pub finished: bool,
    pub played_at: i64,
    pub updated_at: i64,
    pub deleted: bool,
}

/// Valoración de una película o serie: -1, 1 o 2 (ver `reads::RATING_VALUES`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteRating {
    pub profile_id: String,
    pub item_kind: String,
    pub item_key: String,
    pub rating: i64,
    pub updated_at: i64,
    pub deleted: bool,
}

/// Un documento de la nube. Los borrados viajan como documentos con `deleted = true`
/// (borrado lógico): un borrado de verdad no aparecería en la bajada incremental y el
/// otro dispositivo nunca se enteraría.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteDoc {
    Profile(RemoteProfile),
    Favorite(RemoteFavorite),
    Playback(RemotePlayback),
    Rating(RemoteRating),
}

impl RemoteDoc {
    pub fn collection(&self) -> Collection {
        match self {
            RemoteDoc::Profile(_) => Collection::Profiles,
            RemoteDoc::Favorite(_) => Collection::Favorites,
            RemoteDoc::Playback(_) => Collection::Playback,
            RemoteDoc::Rating(_) => Collection::Ratings,
        }
    }

    /// Identidad estable del documento, legible: también es la clave de la cola de
    /// salida (`sync_outbox.target`).
    pub fn target(&self) -> String {
        match self {
            RemoteDoc::Profile(p) => Self::profile_target(&p.id),
            RemoteDoc::Favorite(f) => Self::favorite_target(&f.profile_id, &f.item_kind, &f.item_key),
            RemoteDoc::Playback(p) => Self::playback_target(&p.profile_id, &p.item_kind, &p.item_key),
            RemoteDoc::Rating(r) => Self::rating_target(&r.profile_id, &r.item_kind, &r.item_key),
        }
    }

    pub fn profile_target(profile_id: &str) -> String {
        format!("profile:{profile_id}")
    }

    pub fn favorite_target(profile_id: &str, item_kind: &str, item_key: &str) -> String {
        format!("favorite:{profile_id}:{item_kind}:{item_key}")
    }

    pub fn playback_target(profile_id: &str, item_kind: &str, item_key: &str) -> String {
        format!("playback:{profile_id}:{item_kind}:{item_key}")
    }

    pub fn rating_target(profile_id: &str, item_kind: &str, item_key: &str) -> String {
        format!("rating:{profile_id}:{item_kind}:{item_key}")
    }

    /// Id del documento en la nube. Los perfiles usan su propio id; favoritos y progreso
    /// un hash de `target`, porque `item_key` puede llevar caracteres que un id de
    /// documento no admite (`/`) y longitudes arbitrarias.
    pub fn doc_id(&self) -> String {
        match self {
            RemoteDoc::Profile(p) => p.id.clone(),
            _ => Sha256::digest(self.target().as_bytes()).iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
}

/// Lo que cambió en una colección desde un instante dado.
#[derive(Debug, Default)]
pub struct Changes {
    pub docs: Vec<RemoteDoc>,
    /// Marca de tiempo del servidor (epoch) del cambio más reciente visto, para el
    /// cursor de la siguiente bajada. `None` si no hubo ningún cambio.
    pub max_server_ts: Option<i64>,
}

#[async_trait]
pub trait CloudBackend: Send + Sync {
    async fn sign_up(&self, email: &str, password: &str) -> Result<Session>;
    async fn sign_in(&self, email: &str, password: &str) -> Result<Session>;
    /// Sesión renovada a partir de la actual. `CloudError::SessionExpired` si el token
    /// de refresco ya no vale (revocado, cuenta borrada): hay que volver a entrar.
    async fn refresh(&self, session: &Session) -> Result<Session>;
    /// Manda el correo de recuperación. No revela si el correo tiene cuenta.
    async fn send_password_reset(&self, email: &str) -> Result<()>;
    /// Fija la contraseña nueva con el código (o el enlace entero) del correo.
    async fn confirm_password_reset(&self, code: &str, new_password: &str) -> Result<()>;
    /// Cambia la contraseña con la sesión abierta. Devuelve la sesión nueva: cambiar la
    /// contraseña invalida los tokens anteriores.
    async fn change_password(&self, session: &Session, new_password: &str) -> Result<Session>;
    /// Escribe varios documentos de una vez, todos o ninguno.
    async fn write(&self, session: &Session, docs: &[RemoteDoc]) -> Result<()>;
    /// Documentos de `collection` cambiados en el servidor después de `since` (epoch),
    /// incluidos los borrados lógicos.
    async fn changes_since(&self, session: &Session, collection: Collection, since: i64) -> Result<Changes>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favorite(key: &str) -> RemoteDoc {
        RemoteDoc::Favorite(RemoteFavorite {
            profile_id: "p1".into(),
            item_kind: "movie".into(),
            item_key: key.into(),
            added_at: 1,
            updated_at: 1,
            deleted: false,
        })
    }

    #[test]
    fn doc_id_is_stable_and_safe_for_any_item_key() {
        let a = favorite("matrix-1999");
        assert_eq!(a.doc_id(), favorite("matrix-1999").doc_id());
        assert_ne!(a.doc_id(), favorite("matrix-2003").doc_id());
        let weird = favorite("ac/dc: live / 1991");
        assert!(weird.doc_id().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(weird.doc_id().len(), 64);
    }

    #[test]
    fn payload_roundtrips_through_json_for_the_outbox() {
        let doc = RemoteDoc::Playback(RemotePlayback {
            profile_id: "p1".into(),
            item_kind: "episode".into(),
            item_key: "serie-2020:1:2".into(),
            position_s: 90,
            duration_s: Some(1800),
            finished: false,
            played_at: 10,
            updated_at: 10,
            deleted: false,
        });
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(serde_json::from_str::<RemoteDoc>(&json).unwrap(), doc);
        assert_eq!(doc.target(), "playback:p1:episode:serie-2020:1:2");
    }

    #[test]
    fn only_network_and_server_failures_are_transient() {
        assert!(CloudError::Network.is_transient());
        assert!(CloudError::Unavailable.is_transient());
        assert!(!CloudError::SessionExpired.is_transient());
        assert!(!CloudError::Rejected("x".into()).is_transient());
    }
}
