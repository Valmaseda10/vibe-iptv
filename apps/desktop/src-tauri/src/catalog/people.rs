//! Repartos, fichas de actores y sagas (F17, parte 2).
//!
//! Una consulta a TMDB por título (`/movie/{id}?append_to_response=credits`, que de paso
//! dice a qué saga pertenece la película), en segundo plano y por tandas, los títulos más
//! votados primero (`run_credits_pass`). Se guarda el reparto principal, la dirección (o
//! quien creó la serie) y la saga. La biografía de una persona y las partes de una saga se
//! piden solo al abrir su pantalla.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use super::tmdb::{Result, TmdbClient, TmdbError};
use crate::db::{now_epoch, reads, DbError};

/// Cuántos del reparto se guardan por título (los primeros de los créditos).
const CAST_LIMIT: usize = 15;
/// Títulos por arranque de la app: el catálogo entero se completa en pocos arranques sin
/// gastar de golpe el límite de TMDB (unos 40 por segundo).
const CREDITS_PER_RUN: i64 = 3000;

/// Alguien en los créditos de un título.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonCredit {
    pub id: i64,
    pub name: String,
    pub profile_path: Option<String>,
    pub character: Option<String>,
    pub department: Option<String>,
}

/// La saga de una película, como la da su ficha.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectionRef {
    pub id: i64,
    pub name: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
}

/// Lo que se guarda de un título: reparto en orden, dirección o creación, y su saga.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TitlePeople {
    pub cast: Vec<PersonCredit>,
    pub crew: Vec<PersonCredit>,
    pub collection: Option<CollectionRef>,
}

/// Una película de una saga, esté o no en el catálogo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionPart {
    pub id: i64,
    pub title: String,
    pub release_date: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectionDetail {
    pub id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// En orden de estreno; las que no tienen fecha, al final.
    pub parts: Vec<CollectionPart>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersonDetail {
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
}

// ─────────────── TMDB ───────────────

#[derive(Debug, Default, Deserialize)]
struct CreditsWire {
    #[serde(default)]
    cast: Vec<CastWire>,
    #[serde(default)]
    crew: Vec<CrewWire>,
}

#[derive(Debug, Deserialize)]
struct CastWire {
    id: i64,
    name: String,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    known_for_department: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CrewWire {
    id: i64,
    name: String,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    job: String,
}

#[derive(Debug, Deserialize)]
struct CollectionRefWire {
    id: i64,
    name: String,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MoviePeopleWire {
    #[serde(default)]
    credits: CreditsWire,
    #[serde(default)]
    belongs_to_collection: Option<CollectionRefWire>,
}

#[derive(Debug, Deserialize)]
struct CreatorWire {
    id: i64,
    name: String,
    #[serde(default)]
    profile_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TvPeopleWire {
    #[serde(default)]
    credits: CreditsWire,
    #[serde(default)]
    created_by: Vec<CreatorWire>,
}

#[derive(Debug, Deserialize)]
struct CollectionWire {
    id: i64,
    name: String,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    parts: Vec<CollectionPartWire>,
}

#[derive(Debug, Deserialize)]
struct CollectionPartWire {
    id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PersonWire {
    #[serde(default)]
    biography: Option<String>,
    #[serde(default)]
    birthday: Option<String>,
    #[serde(default)]
    deathday: Option<String>,
    #[serde(default)]
    place_of_birth: Option<String>,
}

/// «Harry Potter - Colección» → «Harry Potter»: la pantalla ya dice que es una saga.
pub fn clean_collection_name(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    for suffix in [" - colección", " – colección", " - collection", " – collection", " (colección)", " colección", " collection"] {
        if lower.ends_with(suffix) {
            let cut = trimmed.len() - suffix.len();
            if trimmed.is_char_boundary(cut) && cut > 0 {
                return trimmed[..cut].trim_end().to_string();
            }
        }
    }
    trimmed.to_string()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn top_cast(mut cast: Vec<CastWire>) -> Vec<PersonCredit> {
    cast.sort_by_key(|c| c.order.unwrap_or(i64::MAX));
    cast.into_iter()
        .take(CAST_LIMIT)
        .map(|c| PersonCredit { id: c.id, name: c.name, profile_path: c.profile_path, character: non_empty(c.character), department: c.known_for_department })
        .collect()
}

impl TmdbClient {
    /// Reparto, dirección y saga de una película, en una sola consulta.
    pub async fn movie_people(&self, tmdb_id: i64) -> Result<TitlePeople> {
        let wire: MoviePeopleWire = self.get_json(&format!("/movie/{tmdb_id}"), &[("language", "es-ES"), ("append_to_response", "credits")]).await?;
        let mut seen = std::collections::HashSet::new();
        let crew = wire
            .credits
            .crew
            .into_iter()
            .filter(|c| c.job == "Director" && seen.insert(c.id))
            .map(|c| PersonCredit { id: c.id, name: c.name, profile_path: c.profile_path, character: None, department: Some("Directing".into()) })
            .collect();
        Ok(TitlePeople {
            cast: top_cast(wire.credits.cast),
            crew,
            collection: wire.belongs_to_collection.map(|c| CollectionRef { id: c.id, name: clean_collection_name(&c.name), poster_path: c.poster_path, backdrop_path: c.backdrop_path }),
        })
    }

    /// Reparto y creadores de una serie, en una sola consulta.
    pub async fn tv_people(&self, tmdb_id: i64) -> Result<TitlePeople> {
        let wire: TvPeopleWire = self.get_json(&format!("/tv/{tmdb_id}"), &[("language", "es-ES"), ("append_to_response", "credits")]).await?;
        Ok(TitlePeople {
            cast: top_cast(wire.credits.cast),
            crew: wire
                .created_by
                .into_iter()
                .map(|c| PersonCredit { id: c.id, name: c.name, profile_path: c.profile_path, character: None, department: Some("Writing".into()) })
                .collect(),
            collection: None,
        })
    }

    /// Una saga con todas sus películas, en orden de estreno.
    pub async fn collection(&self, collection_id: i64) -> Result<CollectionDetail> {
        let wire: CollectionWire = self.get_json(&format!("/collection/{collection_id}"), &[("language", "es-ES")]).await?;
        let mut parts: Vec<CollectionPart> = wire
            .parts
            .into_iter()
            .map(|p| CollectionPart { id: p.id, title: p.title.unwrap_or_default(), release_date: non_empty(p.release_date), poster_path: p.poster_path })
            .collect();
        sort_parts(&mut parts);
        Ok(CollectionDetail { id: wire.id, name: clean_collection_name(&wire.name), overview: non_empty(wire.overview), poster_path: wire.poster_path, backdrop_path: wire.backdrop_path, parts })
    }

    /// Biografía de una persona, en español o, si no la hay, en inglés.
    pub async fn person(&self, person_id: i64) -> Result<PersonDetail> {
        let es: PersonWire = self.get_json(&format!("/person/{person_id}"), &[("language", "es-ES")]).await?;
        let mut biography = non_empty(es.biography);
        if biography.is_none() {
            let en: PersonWire = self.get_json(&format!("/person/{person_id}"), &[("language", "en-US")]).await?;
            biography = non_empty(en.biography);
        }
        Ok(PersonDetail { biography, birthday: non_empty(es.birthday), deathday: non_empty(es.deathday), place_of_birth: non_empty(es.place_of_birth) })
    }
}

/// Orden de estreno; sin fecha (anunciadas), al final.
fn sort_parts(parts: &mut [CollectionPart]) {
    parts.sort_by(|a, b| match (&a.release_date, &b.release_date) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.id.cmp(&b.id),
    });
}

// ─────────────── Base de datos ───────────────

/// Guarda reparto, dirección y saga de un título, y lo marca como comprobado.
pub fn store_title_people(conn: &Connection, item_kind: &str, item_id: i64, people: &TitlePeople) -> crate::db::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM credits WHERE item_kind = ?1 AND item_id = ?2", params![item_kind, item_id])?;
    let crew_role = if item_kind == "series" { "creator" } else { "director" };
    let credits = people.cast.iter().map(|p| ("cast", p)).chain(people.crew.iter().map(|p| (crew_role, p)));
    for (ord, (role, person)) in credits.enumerate() {
        tx.execute(
            "INSERT INTO people (id, name, profile_path, department) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                profile_path = COALESCE(excluded.profile_path, profile_path),
                department = COALESCE(department, excluded.department)",
            params![person.id, person.name, person.profile_path, person.department],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO credits (item_kind, item_id, person_id, role, character, ord) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![item_kind, item_id, person.id, role, person.character, ord as i64],
        )?;
    }
    let table = if item_kind == "series" { "series" } else { "movies" };
    if let Some(collection) = &people.collection {
        tx.execute(
            "INSERT INTO collections (id, name, poster_path, backdrop_path) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                poster_path = COALESCE(excluded.poster_path, poster_path),
                backdrop_path = COALESCE(excluded.backdrop_path, backdrop_path)",
            params![collection.id, collection.name, collection.poster_path, collection.backdrop_path],
        )?;
        tx.execute("UPDATE movies SET collection_id = ?1 WHERE id = ?2", params![collection.id, item_id])?;
    }
    tx.execute(&format!("UPDATE {table} SET credits_checked_at = ?1 WHERE id = ?2"), params![now_epoch(), item_id])?;
    tx.commit()?;
    Ok(())
}

/// Títulos emparejados con TMDB sin reparto pedido, los más votados primero; mitad
/// películas y mitad series.
pub fn list_titles_without_credits(conn: &Connection, limit: i64) -> crate::db::Result<Vec<(&'static str, i64, i64)>> {
    let mut out = Vec::new();
    for (kind, table) in [("movie", "movies"), ("series", "series")] {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, tmdb_id FROM {table} WHERE tmdb_state = 'matched' AND tmdb_id IS NOT NULL AND credits_checked_at IS NULL
             ORDER BY vote_count DESC NULLS LAST, rating DESC NULLS LAST, id LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![(limit + 1) / 2], |row| Ok((kind, row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// Pide a TMDB el reparto (y la saga) de lo que aún no lo tenga, unos miles por arranque.
/// Lo que TMDB ya no conoce se marca sin datos; un fallo de red queda para la próxima vez.
pub async fn run_credits_pass(conn: Connection, client: Arc<TmdbClient>) -> crate::db::Result<u32> {
    let pending = list_titles_without_credits(&conn, CREDITS_PER_RUN)?;
    let conn = Arc::new(AsyncMutex::new(conn));
    let stored = Arc::new(AtomicU32::new(0));
    let mut handles = Vec::with_capacity(pending.len());
    for (kind, id, tmdb_id) in pending {
        let conn = Arc::clone(&conn);
        let client = Arc::clone(&client);
        let stored = Arc::clone(&stored);
        handles.push(tokio::spawn(async move {
            let fetched = if kind == "series" { client.tv_people(tmdb_id).await } else { client.movie_people(tmdb_id).await };
            let people = match fetched {
                Ok(people) => people,
                Err(TmdbError::InvalidResponse(_)) => TitlePeople::default(),
                Err(err) => {
                    tracing::debug!(error = %err, "reparto de TMDB aplazado");
                    return;
                }
            };
            let conn = conn.lock().await;
            match store_title_people(&conn, kind, id, &people) {
                Ok(()) => {
                    stored.fetch_add(1, Ordering::SeqCst);
                }
                Err(err) => tracing::warn!(error = %err, "no se pudo guardar el reparto"),
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    Ok(stored.load(Ordering::SeqCst))
}

/// Alguien en la ficha de un título.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditRow {
    pub person_id: i64,
    pub name: String,
    pub profile_path: Option<String>,
    pub character: Option<String>,
    pub role: String,
}

/// Reparto y dirección (o creación) de un título, en el orden de sus créditos.
pub fn title_credits(conn: &Connection, item_kind: &str, item_id: i64) -> crate::db::Result<Vec<CreditRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.profile_path, c.character, c.role FROM credits c JOIN people p ON p.id = c.person_id
         WHERE c.item_kind = ?1 AND c.item_id = ?2 ORDER BY c.ord",
    )?;
    let rows = stmt
        .query_map(params![item_kind, item_id], |r| Ok(CreditRow { person_id: r.get(0)?, name: r.get(1)?, profile_path: r.get(2)?, character: r.get(3)?, role: r.get(4)? }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Una persona y lo que ha hecho, de lo que hay en el catálogo.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonRow {
    pub id: i64,
    pub name: String,
    pub profile_path: Option<String>,
    pub department: Option<String>,
}

pub fn person(conn: &Connection, person_id: i64) -> crate::db::Result<Option<PersonRow>> {
    conn.query_row("SELECT id, name, profile_path, department FROM people WHERE id = ?1", params![person_id], |r| {
        Ok(PersonRow { id: r.get(0)?, name: r.get(1)?, profile_path: r.get(2)?, department: r.get(3)? })
    })
    .optional()
    .map_err(Into::into)
}

/// Un título de una persona: película o serie, su papel y para ordenar.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonTitle {
    pub item_kind: String,
    pub item_id: i64,
    pub role: String,
    pub character: Option<String>,
}

/// Lo que ha hecho una persona que esté en el catálogo y pueda ver el perfil: lo más
/// votado primero.
pub fn person_titles(conn: &Connection, person_id: i64, access: reads::ProfileAccess) -> crate::db::Result<Vec<PersonTitle>> {
    let mut stmt = conn.prepare(
        "SELECT c.item_kind, c.item_id, c.role, c.character,
                COALESCE(m.vote_count, s.vote_count, 0) AS votes, COALESCE(m.tmdb_id, s.tmdb_id)
         FROM credits c
         LEFT JOIN movies m ON c.item_kind = 'movie' AND m.id = c.item_id
         LEFT JOIN series s ON c.item_kind = 'series' AND s.id = c.item_id
         WHERE c.person_id = ?1
         ORDER BY votes DESC, c.item_id",
    )?;
    let rows = stmt
        .query_map(params![person_id], |r| {
            Ok((PersonTitle { item_kind: r.get(0)?, item_id: r.get(1)?, role: r.get(2)?, character: r.get(3)? }, r.get::<_, Option<i64>>(5)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Un título una vez, aunque salga con dos papeles o dos veces en el catálogo (otra
    // edición de la misma ficha de TMDB).
    let mut seen = std::collections::HashSet::new();
    let mut visible = Vec::new();
    for (title, tmdb_id) in rows {
        let key = (title.item_kind.clone(), tmdb_id.unwrap_or(-title.item_id));
        if seen.contains(&key) || !reads::item_allowed(conn, &title.item_kind, title.item_id, access)? {
            continue;
        }
        seen.insert(key);
        visible.push(title);
    }
    Ok(visible)
}

/// Personas cuyo nombre empieza (o tiene una palabra que empieza) por `query`, con algo en
/// el catálogo; las que más títulos tienen, primero.
pub fn search_people(conn: &Connection, query: &str, limit: i64) -> crate::db::Result<Vec<(PersonRow, i64)>> {
    let query = query.trim();
    if query.chars().count() < 3 {
        return Ok(Vec::new());
    }
    let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.profile_path, p.department, COUNT(DISTINCT c.item_kind || c.item_id) AS titles
         FROM people p JOIN credits c ON c.person_id = p.id
         WHERE p.name LIKE ?1 ESCAPE '\\' OR p.name LIKE ?2 ESCAPE '\\'
         GROUP BY p.id ORDER BY titles DESC, p.name LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![format!("{escaped}%"), format!("% {escaped}%"), limit], |r| {
            Ok((PersonRow { id: r.get(0)?, name: r.get(1)?, profile_path: r.get(2)?, department: r.get(3)? }, r.get(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Biografía guardada de una persona (se pide a TMDB una vez y se guarda).
pub fn cached_person_detail(conn: &Connection, person_id: i64) -> crate::db::Result<Option<PersonDetail>> {
    let raw = crate::db::queries::get_tmdb_cache(conn, &format!("person:{person_id}"))?;
    Ok(raw.and_then(|json| serde_json::from_str(&json).ok()))
}

pub fn store_person_detail(conn: &Connection, person_id: i64, detail: &PersonDetail) -> crate::db::Result<()> {
    let json = serde_json::to_string(detail).map_err(DbError::Json)?;
    crate::db::queries::set_tmdb_cache(conn, &format!("person:{person_id}"), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        crate::db::schema::open_connection_at(std::path::Path::new(":memory:")).unwrap()
    }

    fn person(id: i64, name: &str) -> PersonCredit {
        PersonCredit { id, name: name.into(), profile_path: None, character: Some("Papel".into()), department: Some("Acting".into()) }
    }

    #[test]
    fn parses_movie_people_with_collection_and_directors() {
        let json = r#"{"id":1,"belongs_to_collection":{"id":10,"name":"Saga","poster_path":"/p.jpg","backdrop_path":null},
            "credits":{"cast":[{"id":3,"name":"C","order":2},{"id":2,"name":"B","order":0,"character":" "},{"id":4,"name":"D","order":1,"character":"Neo"}],
                       "crew":[{"id":5,"name":"Dir","job":"Director"},{"id":5,"name":"Dir","job":"Director"},{"id":6,"name":"Prod","job":"Producer"}]}}"#;
        let wire: MoviePeopleWire = serde_json::from_str(json).unwrap();
        let cast = top_cast(wire.credits.cast);
        assert_eq!(cast.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2, 4, 3], "en el orden de los créditos");
        assert_eq!(cast[0].character, None, "un papel vacío no es un papel");
        assert_eq!(wire.belongs_to_collection.unwrap().id, 10);
    }

    #[test]
    fn collection_names_lose_the_collection_suffix() {
        assert_eq!(clean_collection_name("Harry Potter - Colección"), "Harry Potter");
        assert_eq!(clean_collection_name("The Matrix Collection"), "The Matrix");
        assert_eq!(clean_collection_name("Colección"), "Colección", "sin nada delante no se toca");
        assert_eq!(clean_collection_name("Toy Story"), "Toy Story");
    }

    #[test]
    fn parts_go_in_release_order_with_announced_ones_last() {
        let part = |id: i64, date: Option<&str>| CollectionPart { id, title: String::new(), release_date: date.map(Into::into), poster_path: None };
        let mut parts = vec![part(3, None), part(2, Some("2005-01-01")), part(1, Some("1999-03-31"))];
        sort_parts(&mut parts);
        assert_eq!(parts.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn stores_credits_collection_and_finds_people() {
        let conn = memory();
        conn.execute_batch(
            "INSERT INTO profiles (id, name, created_at) VALUES (1, 'Ana', 0);
             INSERT INTO movies (id, clean_title, match_key, tmdb_id, tmdb_state, vote_count) VALUES (1, 'Uno', 'uno-0000', 100, 'matched', 50), (2, 'Dos', 'dos-0000', 200, 'matched', 900),
               (3, 'Dos (edición)', 'dos-edicion-0000', 200, 'matched', 10);",
        )
        .unwrap();
        let people = TitlePeople {
            cast: vec![person(7, "Keanu Reeves"), person(8, "Carrie-Anne Moss")],
            crew: vec![PersonCredit { character: None, ..person(9, "Lana Wachowski") }],
            collection: Some(CollectionRef { id: 55, name: "Matrix".into(), poster_path: None, backdrop_path: None }),
        };
        store_title_people(&conn, "movie", 1, &people).unwrap();
        store_title_people(&conn, "movie", 2, &TitlePeople { cast: vec![person(7, "Keanu Reeves")], ..Default::default() }).unwrap();
        store_title_people(&conn, "movie", 3, &TitlePeople { cast: vec![person(7, "Keanu Reeves")], ..Default::default() }).unwrap();

        let credits = title_credits(&conn, "movie", 1).unwrap();
        assert_eq!(credits.iter().map(|c| (c.person_id, c.role.as_str())).collect::<Vec<_>>(), vec![(7, "cast"), (8, "cast"), (9, "director")]);
        let collection: Option<i64> = conn.query_row("SELECT collection_id FROM movies WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(collection, Some(55));
        assert!(list_titles_without_credits(&conn, 10).unwrap().is_empty(), "ya comprobados");

        let titles = person_titles(&conn, 7, reads::ProfileAccess::UNRESTRICTED).unwrap();
        assert_eq!(titles.iter().map(|t| t.item_id).collect::<Vec<_>>(), vec![2, 1], "lo más votado primero, y la otra edición no se repite");
        let found = search_people(&conn, "reev", 5).unwrap();
        assert_eq!(found.iter().map(|(p, n)| (p.id, *n)).collect::<Vec<_>>(), vec![(7, 3)], "por el apellido también");
        assert!(search_people(&conn, "ke", 5).unwrap().is_empty(), "con menos de tres letras no se busca");

        // Otra ficha de TMDB para el mismo título: reparto y saga fuera, a pedir de nuevo.
        conn.execute("UPDATE movies SET tmdb_id = 101 WHERE id = 1", []).unwrap();
        assert!(title_credits(&conn, "movie", 1).unwrap().is_empty());
        assert_eq!(list_titles_without_credits(&conn, 10).unwrap(), vec![("movie", 1, 101)]);
    }
}
