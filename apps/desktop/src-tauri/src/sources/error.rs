use crate::util::redact;

/// Alias de conveniencia para los métodos de `Source`.
pub type Result<T> = std::result::Result<T, SourceError>;

/// Errores de dominio de la capa de fuentes. Ningún mensaje lleva una URL sin pasar
/// antes por `redact()` (regla R5 de CLAUDE.md).
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("no se pudo contactar con {url}")]
    Unreachable { url: String },

    #[error("credenciales rechazadas por el proveedor")]
    AuthFailed,

    #[error("respuesta inesperada del proveedor en {url}: {detail}")]
    InvalidResponse { url: String, detail: String },

    /// El panel rechaza la petición y dice por qué (p. ej. `{"message": "Validation
    /// failed, Country is not allowed"}`): mejor enseñar su motivo que un "HTTP 403".
    #[error("{0}")]
    Rejected(String),

    #[error("serie o categoría no encontrada: {remote_id}")]
    NotFound { remote_id: String },

    #[error("no se pudo leer la lista M3U: {0}")]
    M3uRead(String),
}

impl SourceError {
    pub fn unreachable(url: &str) -> Self {
        Self::Unreachable { url: redact(url) }
    }

    pub fn invalid_response(url: &str, detail: impl Into<String>) -> Self {
        Self::InvalidResponse {
            url: redact(url),
            detail: detail.into(),
        }
    }
}
