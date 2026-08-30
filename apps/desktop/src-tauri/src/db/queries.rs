//! Consultas tipadas contra SQLite; ningún SQL suelto fuera de este módulo.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::sources::model::{MediaKind, RawCategory, RawChannel, RawMovie, RawSeries};

use super::{now_epoch, Result};

/// Tipo de proveedor, igual que la columna `kind` de `providers` en `docs/DATA-MODEL.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "api-types.ts")]
pub enum ProviderKind {
    Xtream,
    M3u,
}

impl ProviderKind {
    fn as_db_str(self) -> &'static str {
        match self {
            ProviderKind::Xtream => "xtream",
            ProviderKind::M3u => "m3u",
        }
    }

    fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "xtream" => Some(Self::Xtream),
            "m3u" => Some(Self::M3u),
            _ => None,
        }
    }
}

/// Fila cruda de `providers`. `credential_ref` no debe salir de la capa de datos hacia
/// la interfaz: solo hace falta saber que existe, no su valor (regla R5).
#[derive(Debug, Clone)]
pub struct ProviderRow {
    pub id: i64,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub username: Option<String>,
    pub credential_ref: Option<String>,
    pub max_connections: i64,
    pub expires_at: Option<i64>,
    pub enabled: bool,
    pub last_sync_at: Option<i64>,
    pub created_at: i64,
}

fn row_to_provider(row: &Row) -> rusqlite::Result<ProviderRow> {
    let kind_str: String = row.get("kind")?;
    Ok(ProviderRow {
        id: row.get("id")?,
        name: row.get("name")?,
        kind: ProviderKind::from_db_str(&kind_str).unwrap_or(ProviderKind::Xtream),
        base_url: row.get("base_url")?,
        username: row.get("username")?,
        credential_ref: row.get("credential_ref")?,
        max_connections: row.get("max_connections")?,
        expires_at: row.get("expires_at")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        last_sync_at: row.get("last_sync_at")?,
        created_at: row.get("created_at")?,
    })
}

pub fn insert_provider(
    conn: &Connection,
    kind: ProviderKind,
    name: &str,
    base_url: &str,
    username: Option<&str>,
    max_connections: i64,
    expires_at: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO providers (name, kind, base_url, username, max_connections, expires_at, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
        params![name, kind.as_db_str(), base_url, username, max_connections, expires_at, now_epoch()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_provider_credential_ref(conn: &Connection, provider_id: i64, credential_ref: &str) -> Result<()> {
    conn.execute(
        "UPDATE providers SET credential_ref = ?1 WHERE id = ?2",
        params![credential_ref, provider_id],
    )?;
    Ok(())
}

pub fn update_provider_account_status(
    conn: &Connection,
    provider_id: i64,
    max_connections: i64,
    expires_at: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE providers SET max_connections = ?1, expires_at = ?2 WHERE id = ?3",
        params![max_connections, expires_at, provider_id],
    )?;
    Ok(())
}

pub fn mark_provider_synced(conn: &Connection, provider_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE providers SET last_sync_at = ?1 WHERE id = ?2",
        params![now_epoch(), provider_id],
    )?;
    Ok(())
}

pub fn get_provider(conn: &Connection, id: i64) -> Result<Option<ProviderRow>> {
    conn.query_row("SELECT * FROM providers WHERE id = ?1", params![id], row_to_provider)
        .optional()
        .map_err(Into::into)
}

pub fn list_providers(conn: &Connection) -> Result<Vec<ProviderRow>> {
    let mut stmt = conn.prepare("SELECT * FROM providers ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], row_to_provider)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn media_kind_db(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Live => "live",
        MediaKind::Movie => "movie",
        MediaKind::Series => "series",
    }
}

/// Upsert de categorías de un tipo, y mapa `remote_id -> id local` para enlazar
/// canales/películas con su categoría al insertarlos.
pub fn upsert_categories(
    conn: &Connection,
    provider_id: i64,
    kind: MediaKind,
    categories: &[RawCategory],
) -> Result<HashMap<String, i64>> {
    let kind_str = media_kind_db(kind);
    {
        let mut stmt = conn.prepare(
            "INSERT INTO categories (provider_id, remote_id, kind, name) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id, kind, remote_id) DO UPDATE SET name = excluded.name",
        )?;
        for category in categories {
            stmt.execute(params![provider_id, category.remote_id, kind_str, category.name])?;
        }
    }

    let mut map = HashMap::new();
    let mut select = conn.prepare("SELECT id, remote_id FROM categories WHERE provider_id = ?1 AND kind = ?2")?;
    let mut rows = select.query(params![provider_id, kind_str])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let remote_id: String = row.get(1)?;
        map.insert(remote_id, id);
    }
    Ok(map)
}

/// `display_name` se deja igual a `raw_name`: la limpieza real de títulos es de la fase
/// F3 (`catalog::normalize`), que actualizará esta columna sobre las filas existentes.
pub fn upsert_channel(
    conn: &Connection,
    provider_id: i64,
    category_map: &HashMap<String, i64>,
    channel: &RawChannel,
) -> Result<()> {
    let category_id = channel.category_remote_id.as_ref().and_then(|id| category_map.get(id).copied());
    conn.execute(
        "INSERT INTO channels
            (provider_id, remote_id, category_id, raw_name, display_name, logo_url, epg_channel_id,
             has_catchup, catchup_days, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            category_id = excluded.category_id,
            raw_name = excluded.raw_name,
            display_name = excluded.display_name,
            logo_url = excluded.logo_url,
            epg_channel_id = excluded.epg_channel_id,
            has_catchup = excluded.has_catchup,
            catchup_days = excluded.catchup_days,
            sort_order = excluded.sort_order",
        params![
            provider_id,
            channel.remote_id,
            category_id,
            channel.raw_name,
            channel.raw_name,
            channel.logo_url,
            channel.epg_channel_id,
            channel.has_catchup as i64,
            channel.catchup_days,
            channel.sort_order,
        ],
    )?;
    Ok(())
}

/// `clean_title`/`match_key` son un placeholder (título tal cual, sin año) hasta que
/// F3 implemente `catalog::normalize`. Agrupa por título exacto: no está mal, solo es
/// menos preciso que la normalización real.
pub fn upsert_movie(
    conn: &Connection,
    provider_id: i64,
    category_map: &HashMap<String, i64>,
    movie: &RawMovie,
) -> Result<()> {
    let clean_title = movie.raw_title.trim().to_string();
    let match_key = format!("{}-0000", placeholder_slug(&clean_title));

    conn.execute(
        "INSERT INTO movies (clean_title, match_key, tmdb_id, added_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(match_key) DO UPDATE SET clean_title = excluded.clean_title",
        params![clean_title, match_key, movie.tmdb_id, movie.added_at],
    )?;
    let movie_id: i64 = conn.query_row("SELECT id FROM movies WHERE match_key = ?1", params![match_key], |row| {
        row.get(0)
    })?;

    let category_id = movie.category_remote_id.as_ref().and_then(|id| category_map.get(id).copied());
    conn.execute(
        "INSERT INTO movie_sources (movie_id, provider_id, remote_id, category_id, raw_title, container)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            movie_id = excluded.movie_id,
            category_id = excluded.category_id,
            raw_title = excluded.raw_title,
            container = excluded.container",
        params![movie_id, provider_id, movie.remote_id, category_id, movie.raw_title, movie.container_extension],
    )?;
    Ok(())
}

/// Mismo placeholder de agrupación que [`upsert_movie`]; `series_sources` no tiene
/// columna de categoría (ver `docs/DATA-MODEL.md`).
pub fn upsert_series(conn: &Connection, provider_id: i64, series: &RawSeries) -> Result<()> {
    let clean_title = series.raw_title.trim().to_string();
    let match_key = format!("{}-0000", placeholder_slug(&clean_title));

    conn.execute(
        "INSERT INTO series (clean_title, match_key, added_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(match_key) DO UPDATE SET clean_title = excluded.clean_title",
        params![clean_title, match_key, now_epoch()],
    )?;
    let series_id: i64 = conn.query_row("SELECT id FROM series WHERE match_key = ?1", params![match_key], |row| {
        row.get(0)
    })?;

    conn.execute(
        "INSERT INTO series_sources (series_id, provider_id, remote_id, raw_title)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(provider_id, remote_id) DO UPDATE SET
            series_id = excluded.series_id,
            raw_title = excluded.raw_title",
        params![series_id, provider_id, series.remote_id, series.raw_title],
    )?;
    Ok(())
}

fn placeholder_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for ch in input.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn count_categories(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM categories WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

pub fn count_channels(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM channels WHERE provider_id = ?1", params![provider_id], |row| row.get(0))
        .map_err(Into::into)
}

pub fn count_movie_sources(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM movie_sources WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

pub fn count_series_sources(conn: &Connection, provider_id: i64) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM series_sources WHERE provider_id = ?1", params![provider_id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_connection_at;
    use std::path::Path;

    fn memory_conn() -> Connection {
        open_connection_at(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn insert_and_list_provider_roundtrip() {
        let conn = memory_conn();
        let id = insert_provider(&conn, ProviderKind::Xtream, "Mi proveedor", "http://panel.example.com:8080", Some("demo"), 2, Some(1893456000)).unwrap();

        let providers = list_providers(&conn).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, id);
        assert_eq!(providers[0].kind, ProviderKind::Xtream);
        assert_eq!(providers[0].max_connections, 2);
        assert!(providers[0].credential_ref.is_none());

        set_provider_credential_ref(&conn, id, "provider-1").unwrap();
        let fetched = get_provider(&conn, id).unwrap().unwrap();
        assert_eq!(fetched.credential_ref.as_deref(), Some("provider-1"));
    }

    #[test]
    fn upsert_categories_returns_remote_id_map_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let cats = vec![
            RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() },
            RawCategory { remote_id: "2".into(), kind: MediaKind::Live, name: "DOCUS".into() },
        ];
        let map1 = upsert_categories(&conn, provider_id, MediaKind::Live, &cats).unwrap();
        assert_eq!(map1.len(), 2);

        // Repetir con el mismo remote_id pero nombre distinto: actualiza, no duplica.
        let cats2 = vec![RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA HD".into() }];
        let map2 = upsert_categories(&conn, provider_id, MediaKind::Live, &cats2).unwrap();
        assert_eq!(map2["1"], map1["1"]);

        let total = count_categories(&conn, provider_id).unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn upsert_channel_links_category_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let cats = vec![RawCategory { remote_id: "1".into(), kind: MediaKind::Live, name: "ESPAÑA".into() }];
        let category_map = upsert_categories(&conn, provider_id, MediaKind::Live, &cats).unwrap();

        let channel = RawChannel {
            remote_id: "101".into(),
            category_remote_id: Some("1".into()),
            raw_name: "Canal Uno".into(),
            logo_url: None,
            epg_channel_id: None,
            has_catchup: true,
            catchup_days: 7,
            sort_order: Some(1),
        };
        upsert_channel(&conn, provider_id, &category_map, &channel).unwrap();
        upsert_channel(&conn, provider_id, &category_map, &channel).unwrap(); // repetir: no duplica

        assert_eq!(count_channels(&conn, provider_id).unwrap(), 1);
    }

    #[test]
    fn upsert_movie_groups_by_placeholder_match_key() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let category_map = HashMap::new();

        let movie_a = RawMovie {
            remote_id: "5001".into(),
            category_remote_id: None,
            raw_title: "Mi Película".into(),
            container_extension: Some("mkv".into()),
            logo_url: None,
            added_at: Some(1700000000),
            rating: Some(7.5),
            tmdb_id: None,
        };
        // Misma película, otro proveedor/remote_id, mismo título exacto: debe agruparse
        // bajo la misma fila `movies` (comportamiento esperado del placeholder de F2).
        let movie_b = RawMovie { remote_id: "9999".into(), ..movie_a.clone() };

        upsert_movie(&conn, provider_id, &category_map, &movie_a).unwrap();
        upsert_movie(&conn, provider_id, &category_map, &movie_b).unwrap();

        let movies_count: i64 = conn.query_row("SELECT COUNT(*) FROM movies", [], |r| r.get(0)).unwrap();
        assert_eq!(movies_count, 1);
        assert_eq!(count_movie_sources(&conn, provider_id).unwrap(), 2);
    }

    #[test]
    fn upsert_series_has_no_category_column_and_is_idempotent() {
        let conn = memory_conn();
        let provider_id = insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();

        let series = RawSeries {
            remote_id: "9001".into(),
            raw_title: "Serie de Ejemplo".into(),
            cover_url: None,
            plot: None,
            release_date: None,
        };
        upsert_series(&conn, provider_id, &series).unwrap();
        upsert_series(&conn, provider_id, &series).unwrap();

        assert_eq!(count_series_sources(&conn, provider_id).unwrap(), 1);
    }
}
