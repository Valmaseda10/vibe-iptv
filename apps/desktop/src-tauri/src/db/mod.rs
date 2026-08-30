pub mod queries;
pub mod schema;

/// Errores de la capa de persistencia.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("no se pudo resolver el directorio de datos: {0}")]
    DataDir(String),
    #[error("error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;

pub(crate) fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
