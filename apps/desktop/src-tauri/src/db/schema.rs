//! Conexión SQLite y aplicación de migraciones. Las migraciones se embeben en el
//! binario en tiempo de compilación (`include_str!`): así no dependen de que
//! `migrations/` viaje como recurso empaquetado aparte.

use std::path::Path;

use rusqlite::Connection;

use super::{DbError, Result};

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_schema_inicial.sql", include_str!("../../migrations/001_schema_inicial.sql")),
    ("002_duracion_probe_cache.sql", include_str!("../../migrations/002_duracion_probe_cache.sql")),
    ("003_lan_pairing.sql", include_str!("../../migrations/003_lan_pairing.sql")),
    ("004_perfiles_netflix.sql", include_str!("../../migrations/004_perfiles_netflix.sql")),
    ("005_categoria_series.sql", include_str!("../../migrations/005_categoria_series.sql")),
    ("006_trailers.sql", include_str!("../../migrations/006_trailers.sql")),
    ("007_generos_y_clasificacion.sql", include_str!("../../migrations/007_generos_y_clasificacion.sql")),
    ("008_info_tecnica_vod.sql", include_str!("../../migrations/008_info_tecnica_vod.sql")),
    ("009_titulos_similares.sql", include_str!("../../migrations/009_titulos_similares.sql")),
    ("010_pistas_de_audio.sql", include_str!("../../migrations/010_pistas_de_audio.sql")),
    ("011_cola_sync_nube.sql", include_str!("../../migrations/011_cola_sync_nube.sql")),
    ("012_caratulas_proveedor.sql", include_str!("../../migrations/012_caratulas_proveedor.sql")),
];

/// Ruta del fichero SQLite en el directorio de datos de la aplicación, sin abrirlo.
pub fn resolve_db_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    use tauri::Manager;

    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|err| DbError::DataDir(err.to_string()))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("vibe-iptv.sqlite3"))
}

/// Abre (o crea) la base de datos en el directorio de datos de la aplicación.
pub fn open_connection(app_handle: &tauri::AppHandle) -> Result<Connection> {
    open_connection_at(&resolve_db_path(app_handle)?)
}

/// Igual que [`open_connection`] pero sobre una ruta explícita; separado para poder
/// probarlo sin un `AppHandle` real.
pub fn open_connection_at(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    run_migrations(&mut conn)?;
    Ok(conn)
}

/// Para lecturas frecuentes (la API HTTP): sin comprobar migraciones en cada apertura.
pub fn open_readonly_at(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

/// Con WAL hay un único escritor a la vez. Sin espera, el segundo falla al instante con
/// `SQLITE_BUSY`; desde F11 el motor de sincronización escribe en segundo plano mientras
/// la API guarda progreso, así que la colisión deja de ser rara. Los bloqueos reales
/// duran milisegundos: cinco segundos es un margen holgado, no una espera esperada.
fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            filename    TEXT PRIMARY KEY,
            applied_at  INTEGER NOT NULL
        );",
    )?;

    for (filename, sql) in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM _migrations WHERE filename = ?1)",
            [filename],
            |row| row.get(0),
        )?;
        if already_applied {
            continue;
        }

        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO _migrations (filename, applied_at) VALUES (?1, ?2)",
            rusqlite::params![filename, super::now_epoch()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_migrations_and_creates_expected_tables() {
        let conn = open_connection_at(Path::new(":memory:")).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'providers'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'search_index'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn reopening_and_migrating_again_is_a_no_op() {
        let path = std::env::temp_dir().join(format!("vibe-iptv-test-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);

        open_connection_at(&path).unwrap();
        // Segunda apertura sobre el mismo fichero: no debe fallar por "table already exists".
        open_connection_at(&path).unwrap();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn wal_and_foreign_keys_are_enabled() {
        let path = std::env::temp_dir().join(format!("vibe-iptv-test-pragmas-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = open_connection_at(&path).unwrap();

        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
        assert_eq!(foreign_keys, 1);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
