//! Ajustes → Datos y copia de seguridad (2026-11-02): cuánto hay en el catálogo, cuánto
//! ocupa la caché de portadas (y vaciarla), y exportar o importar la copia de seguridad.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::db::schema;
use crate::sync::backup::{self, BackupFile, BackupSummary};

/// Lo que hay en este aparato.
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct LibraryStats {
    #[ts(type = "number")]
    pub channels: i64,
    #[ts(type = "number")]
    pub movies: i64,
    #[ts(type = "number")]
    pub series: i64,
    #[ts(type = "number")]
    pub episodes: i64,
    #[ts(type = "number")]
    pub image_cache_bytes: u64,
    #[ts(type = "number")]
    pub image_cache_files: u64,
}

fn image_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|_| "no se pudo resolver el directorio de datos".to_string())?.join("images"))
}

fn dir_usage(dir: &Path) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return (0, 0) };
    entries
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .fold((0, 0), |(bytes, files), m| (bytes + m.len(), files + 1))
}

/// Comando Tauri: tamaño del catálogo y de la caché de portadas.
#[tauri::command]
pub fn library_stats(app: AppHandle) -> Result<LibraryStats, String> {
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map_err(|err| err.to_string());
    let (image_cache_bytes, image_cache_files) = dir_usage(&image_cache_dir(&app)?);
    Ok(LibraryStats {
        channels: count("SELECT COUNT(*) FROM channels")?,
        movies: count("SELECT COUNT(*) FROM movies")?,
        series: count("SELECT COUNT(*) FROM series")?,
        episodes: count("SELECT COUNT(*) FROM episodes")?,
        image_cache_bytes,
        image_cache_files,
    })
}

/// Comando Tauri: borra las portadas guardadas (se vuelven a bajar según se ven). Devuelve
/// los bytes liberados.
#[tauri::command]
pub fn clear_image_cache(app: AppHandle) -> Result<u64, String> {
    let dir = image_cache_dir(&app)?;
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(0) };
    let mut freed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() && std::fs::remove_file(entry.path()).is_ok() {
            freed += meta.len();
        }
    }
    tracing::info!(freed, "caché de portadas vaciada a mano");
    Ok(freed)
}

/// Comando Tauri: guarda la copia de seguridad en Documentos (en el iPhone, en la carpeta
/// de la app que se ve en «Archivos») y devuelve dónde.
#[tauri::command]
pub fn export_backup(app: AppHandle, ui: serde_json::Value) -> Result<String, String> {
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    let version = app.package_info().version.to_string();
    let file = backup::export(&conn, &version, ui).map_err(|err| err.to_string())?;
    let json = serde_json::to_string_pretty(&file).map_err(|err| err.to_string())?;
    let dir = app.path().document_dir().map_err(|_| "no se encontró la carpeta Documentos".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let day = time::OffsetDateTime::now_utc().date();
    let path = dir.join(format!("VIBE IPTV copia {day}.json"));
    std::fs::write(&path, json).map_err(|err| err.to_string())?;
    Ok(path.display().to_string())
}

/// Comando Tauri: mete en este aparato una copia de seguridad (el contenido del fichero).
#[tauri::command]
pub fn import_backup(app: AppHandle, contents: String) -> Result<BackupSummary, String> {
    let file: BackupFile = serde_json::from_str(&contents).map_err(|_| backup::BackupError::NotABackup.to_string())?;
    let conn = schema::open_connection(&app).map_err(|err| err.to_string())?;
    backup::import(&conn, file).map_err(|err| err.to_string())
}
