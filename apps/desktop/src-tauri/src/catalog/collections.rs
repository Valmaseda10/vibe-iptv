//! Sagas (F17, parte 2): todas las películas de una saga en orden, con lo visto marcado y
//! la siguiente que toca. La saga de cada película la guarda `catalog::people` al pedir su
//! reparto; la lista completa (también las películas que el catálogo no tiene) se pide a
//! TMDB al abrir la saga y se guarda un mes.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use super::people::{CollectionDetail, CollectionPart};
use crate::db::{now_epoch, reads, Result};

/// La lista de partes de una saga se vuelve a pedir pasado este tiempo (salen películas).
pub const PARTS_MAX_AGE_S: i64 = 30 * 24 * 3600;

/// Una saga, para una fila o para el bloque de la ficha de una película.
#[derive(Debug, Clone, PartialEq)]
pub struct SagaSummary {
    pub id: i64,
    pub name: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// Películas de la saga que hay en el catálogo y puede ver el perfil.
    pub available: i64,
    /// De esas, las que el perfil ha terminado.
    pub watched: i64,
}

/// Una película de la saga, en su orden.
#[derive(Debug, Clone, PartialEq)]
pub struct SagaEntry {
    pub title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    /// En el catálogo (y la puede ver el perfil): su id. Si no, la saga la enseña en gris.
    pub movie_id: Option<i64>,
    pub watched: bool,
    /// Lo visto (0–1) si se dejó a medias.
    pub progress: Option<f64>,
}

/// Lo guardado de una saga: nombre, imágenes y, si se pidieron, sus partes.
#[derive(Debug, Clone, PartialEq)]
pub struct SagaHeader {
    pub id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub parts: Option<Vec<CollectionPart>>,
    pub parts_fetched_at: Option<i64>,
}

pub fn header(conn: &Connection, collection_id: i64) -> Result<Option<SagaHeader>> {
    conn.query_row(
        "SELECT id, name, overview, poster_path, backdrop_path, parts_json, parts_fetched_at FROM collections WHERE id = ?1",
        params![collection_id],
        |r| {
            let parts_json: Option<String> = r.get(5)?;
            let name: String = r.get(1)?;
            Ok(SagaHeader {
                id: r.get(0)?,
                name: super::people::clean_collection_name(&name),
                overview: r.get(2)?,
                poster_path: r.get(3)?,
                backdrop_path: r.get(4)?,
                parts: parts_json.and_then(|json| serde_json::from_str(&json).ok()),
                parts_fetched_at: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Si hay que (volver a) pedir a TMDB las partes de la saga.
pub fn parts_stale(header: &SagaHeader, now: i64) -> bool {
    header.parts_fetched_at.is_none_or(|at| now - at > PARTS_MAX_AGE_S)
}

/// Guarda lo que dio TMDB de la saga.
pub fn store_detail(conn: &Connection, detail: &CollectionDetail) -> Result<()> {
    let parts_json = serde_json::to_string(&detail.parts)?;
    conn.execute(
        "INSERT INTO collections (id, name, overview, poster_path, backdrop_path, parts_json, parts_fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, overview = excluded.overview,
            poster_path = COALESCE(excluded.poster_path, poster_path),
            backdrop_path = COALESCE(excluded.backdrop_path, backdrop_path),
            parts_json = excluded.parts_json, parts_fetched_at = excluded.parts_fetched_at",
        params![detail.id, detail.name, detail.overview, detail.poster_path, detail.backdrop_path, parts_json, now_epoch()],
    )?;
    Ok(())
}

/// Una película de la saga en el catálogo.
struct LocalPart {
    movie_id: i64,
    tmdb_id: Option<i64>,
    title: String,
    year: Option<i32>,
    poster_path: Option<String>,
    finished: bool,
    progress: Option<f64>,
}

fn local_parts(conn: &Connection, collection_id: i64, profile_id: i64, access: reads::ProfileAccess) -> Result<Vec<LocalPart>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.tmdb_id, m.clean_title, m.year, COALESCE(m.poster_path, m.provider_poster_url),
                COALESCE(p.finished, 0), p.position_s, p.duration_s
         FROM movies m
         LEFT JOIN playback_state p ON p.profile_id = ?2 AND p.item_kind = 'movie' AND p.item_id = m.id
         WHERE m.collection_id = ?1
         ORDER BY m.year NULLS LAST, m.id",
    )?;
    let rows = stmt
        .query_map(params![collection_id, profile_id], |r| {
            let position: Option<i64> = r.get(6)?;
            let duration: Option<i64> = r.get(7)?;
            Ok(LocalPart {
                movie_id: r.get(0)?,
                tmdb_id: r.get(1)?,
                title: r.get(2)?,
                year: r.get(3)?,
                poster_path: r.get(4)?,
                finished: r.get::<_, i64>(5)? != 0,
                progress: match (position, duration) {
                    (Some(p), Some(d)) if d > 0 && p > 0 => Some((p as f64 / d as f64).min(1.0)),
                    _ => None,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut visible: Vec<LocalPart> = Vec::new();
    for part in rows {
        if !reads::item_allowed(conn, "movie", part.movie_id, access)? {
            continue;
        }
        // La misma película dos veces en el catálogo (otra edición, otro año en el título):
        // una sola entrada, vista si se vio cualquiera de las dos.
        if let Some(same) = part.tmdb_id.and_then(|tmdb_id| visible.iter_mut().find(|p| p.tmdb_id == Some(tmdb_id))) {
            same.finished |= part.finished;
            same.progress = match (same.progress, part.progress) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            continue;
        }
        visible.push(part);
    }
    Ok(visible)
}

fn entry_from_local(part: &LocalPart) -> SagaEntry {
    SagaEntry {
        title: part.title.clone(),
        year: part.year,
        poster_path: part.poster_path.clone(),
        movie_id: Some(part.movie_id),
        watched: part.finished,
        progress: if part.finished { None } else { part.progress },
    }
}

/// Las películas de la saga en orden. Con las partes de TMDB, también las que el
/// catálogo no tiene (sin `movie_id`); sin ellas, solo las del catálogo, por año.
pub fn entries(conn: &Connection, collection_id: i64, profile_id: i64, access: reads::ProfileAccess, parts: Option<&[CollectionPart]>) -> Result<Vec<SagaEntry>> {
    let local = local_parts(conn, collection_id, profile_id, access)?;
    let Some(parts) = parts else {
        return Ok(local.iter().map(entry_from_local).collect());
    };
    let mut by_tmdb: HashMap<i64, &LocalPart> = HashMap::new();
    for part in &local {
        if let Some(tmdb_id) = part.tmdb_id {
            by_tmdb.entry(tmdb_id).or_insert(part);
        }
    }
    let mut used = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        match by_tmdb.get(&part.id) {
            Some(local) => {
                used.insert(local.movie_id);
                out.push(entry_from_local(local));
            }
            None => out.push(SagaEntry {
                title: part.title.clone(),
                year: part.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()),
                poster_path: part.poster_path.clone(),
                movie_id: None,
                watched: false,
                progress: None,
            }),
        }
    }
    // Una del catálogo que TMDB no pone en la lista (pasa con montajes especiales): al final.
    out.extend(local.iter().filter(|p| !used.contains(&p.movie_id)).map(entry_from_local));
    Ok(out)
}

/// La que toca ver: la primera disponible sin terminar, empezando por las que van después
/// de `after` (la película que se acaba de ver) si se da.
pub fn next_to_watch(entries: &[SagaEntry], after: Option<i64>) -> Option<i64> {
    let start = after.and_then(|id| entries.iter().position(|e| e.movie_id == Some(id))).map_or(0, |i| i + 1);
    let candidate = |e: &&SagaEntry| e.movie_id.is_some() && !e.watched && e.movie_id != after;
    entries[start..].iter().find(candidate).or_else(|| if after.is_some() { None } else { entries.iter().find(candidate) }).and_then(|e| e.movie_id)
}

/// La saga de una película, si tiene y hay al menos otra película suya en el catálogo.
pub fn movie_saga(conn: &Connection, movie_id: i64, profile_id: i64, access: reads::ProfileAccess) -> Result<Option<SagaSummary>> {
    let Some(collection_id) = conn.query_row("SELECT collection_id FROM movies WHERE id = ?1", params![movie_id], |r| r.get::<_, Option<i64>>(0)).optional()?.flatten() else {
        return Ok(None);
    };
    let Some(header) = header(conn, collection_id)? else { return Ok(None) };
    let local = local_parts(conn, collection_id, profile_id, access)?;
    let total = header.parts.as_ref().map_or(local.len(), |parts| parts.len().max(local.len()));
    if total < 2 {
        return Ok(None);
    }
    Ok(Some(SagaSummary {
        id: header.id,
        name: header.name,
        poster_path: header.poster_path,
        backdrop_path: header.backdrop_path,
        available: local.len() as i64,
        watched: local.iter().filter(|p| p.finished).count() as i64,
    }))
}

/// Sagas con al menos dos películas en el catálogo que pueda ver el perfil, las más
/// populares primero (suma de votos de TMDB de sus películas).
pub fn saga_row(conn: &Connection, profile_id: i64, access: reads::ProfileAccess, limit: usize) -> Result<Vec<SagaSummary>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.poster_path, c.backdrop_path
         FROM collections c JOIN movies m ON m.collection_id = c.id
         GROUP BY c.id HAVING COUNT(*) >= 2
         ORDER BY SUM(COALESCE(m.vote_count, 0)) DESC, c.id
         LIMIT ?1",
    )?;
    // Algunas se caen por el perfil (infantil): se piden de más.
    let candidates = stmt
        .query_map(params![(limit * 3) as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String, Option<String>, Option<String>)>>>()?;
    let mut out = Vec::new();
    for (id, name, poster_path, backdrop_path) in candidates {
        let local = local_parts(conn, id, profile_id, access)?;
        if local.len() < 2 {
            continue;
        }
        let name = super::people::clean_collection_name(&name);
        out.push(SagaSummary { id, name, poster_path, backdrop_path, available: local.len() as i64, watched: local.iter().filter(|p| p.finished).count() as i64 });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> Connection {
        let conn = crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO profiles (id, name, created_at) VALUES (1, 'Ana', 0);
             INSERT INTO collections (id, name) VALUES (55, 'Matrix: colección'), (66, 'Sola');
             INSERT INTO movies (id, clean_title, match_key, year, tmdb_id, collection_id, vote_count) VALUES
               (1, 'Matrix', 'matrix-1999', 1999, 603, 55, 100),
               (2, 'Matrix (montaje)', 'matrix-montaje-1999', 1999, 603, 55, 1),
               (3, 'Matrix Revolutions', 'matrix-revolutions-2003', 2003, 605, 55, 50),
               (9, 'Única', 'unica-2000', 2000, 900, 66, 10);
             INSERT INTO playback_state (profile_id, item_kind, item_id, position_s, duration_s, finished, played_at) VALUES (1, 'movie', 1, 7000, 7000, 1, 0);",
        )
        .unwrap();
        conn
    }

    fn part(id: i64, title: &str, date: &str) -> CollectionPart {
        CollectionPart { id, title: title.into(), release_date: Some(date.into()), poster_path: None }
    }

    #[test]
    fn entries_follow_tmdb_order_and_show_missing_parts() {
        let conn = seed();
        let all = reads::ProfileAccess::UNRESTRICTED;
        let parts = vec![part(603, "Matrix", "1999-03-31"), part(604, "Matrix Reloaded", "2003-05-15"), part(605, "Matrix Revolutions", "2003-11-05")];
        let list = entries(&conn, 55, 1, all, Some(&parts)).unwrap();
        assert_eq!(list.iter().map(|e| (e.movie_id, e.watched)).collect::<Vec<_>>(), vec![(Some(1), true), (None, false), (Some(3), false)]);
        assert_eq!(list[1].year, Some(2003));
        assert_eq!(next_to_watch(&list, None), Some(3), "la primera sin ver que se puede ver");
        assert_eq!(next_to_watch(&list, Some(1)), Some(3), "al acabar la 1, la siguiente que hay");
        assert_eq!(next_to_watch(&list, Some(3)), None, "tras la última no se vuelve a empezar");

        let without_parts = entries(&conn, 55, 1, all, None).unwrap();
        assert_eq!(without_parts.iter().map(|e| e.movie_id).collect::<Vec<_>>(), vec![Some(1), Some(3)]);
    }

    #[test]
    fn rows_and_badges_need_two_movies_of_the_saga() {
        let conn = seed();
        let all = reads::ProfileAccess::UNRESTRICTED;
        let row = saga_row(&conn, 1, all, 10).unwrap();
        assert_eq!(row.iter().map(|s| (s.id, s.available, s.watched)).collect::<Vec<_>>(), vec![(55, 2, 1)]);
        assert!(movie_saga(&conn, 1, 1, all).unwrap().is_some());
        assert!(movie_saga(&conn, 9, 1, all).unwrap().is_none(), "una saga de una sola película no es saga");
        let header = header(&conn, 55).unwrap().unwrap();
        assert!(parts_stale(&header, now_epoch()));
        store_detail(&conn, &CollectionDetail { id: 55, name: "Matrix".into(), overview: None, poster_path: None, backdrop_path: None, parts: vec![part(603, "Matrix", "1999-03-31")] }).unwrap();
        let header = super::header(&conn, 55).unwrap().unwrap();
        assert!(!parts_stale(&header, now_epoch()));
        assert_eq!(header.parts.unwrap().len(), 1);
    }
}
