//! Copia de seguridad en un fichero (2026-11-02): perfiles, Mi lista, valoraciones,
//! progreso, equipos seguidos, recordatorios y tiempo visto, para pasarlos a otro aparato
//! sin cuenta en la nube. Usa los mismos documentos que la nube (`RemoteDoc`), así que
//! importar es lo mismo que recibirlos de ella: gana lo más reciente y lo que este
//! catálogo no tiene se salta.
//!
//! Nunca lleva credenciales ni direcciones de proveedores ni la clave de TMDB (R5).

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use super::backend::RemoteDoc;
use super::outbox;
use crate::db::{cloud, now_epoch, reads};

/// Marca del formato, para no confundir este fichero con cualquier otro JSON.
pub const FORMAT: &str = "vibe-iptv-backup";
const VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("Este fichero no es una copia de seguridad de VIBE IPTV.")]
    NotABackup,
    #[error("La copia es de una versión más nueva de VIBE IPTV: actualiza la app primero.")]
    TooNew,
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

/// El fichero entero. `ui` lo rellena y lo lee la interfaz (subtítulos y preferencias de
/// reproducción, por nombre de perfil): el núcleo lo guarda tal cual.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub exported_at: i64,
    pub docs: Vec<RemoteDoc>,
    #[serde(default)]
    pub ui: serde_json::Value,
}

/// Lo que trajo una importación, para enseñarlo.
#[derive(Debug, Default, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "api-types.ts")]
pub struct BackupSummary {
    #[ts(type = "number")]
    pub profiles: i64,
    #[ts(type = "number")]
    pub my_list: i64,
    #[ts(type = "number")]
    pub ratings: i64,
    #[ts(type = "number")]
    pub progress: i64,
    #[ts(type = "number")]
    pub follows: i64,
    #[ts(type = "number")]
    pub reminders: i64,
    /// Títulos o canales de la copia que este catálogo no tiene.
    #[ts(type = "number")]
    pub not_found: i64,
}

/// Todo lo de los perfiles de este aparato, como documentos.
pub fn export(conn: &Connection, app_version: &str, ui: serde_json::Value) -> Result<BackupFile, BackupError> {
    let mut docs = Vec::new();
    let device = crate::catalog::watch_time::device_id(conn)?;
    let now = now_epoch();
    for profile in cloud::sync_profiles(conn)? {
        let id = profile.id;
        docs.extend(outbox::profile_doc(conn, id)?);
        for (kind, item_id) in favorites(conn, id)? {
            docs.extend(outbox::favorite_doc(conn, id, &kind, item_id)?);
        }
        for rating in reads::list_ratings(conn, id)? {
            docs.extend(outbox::rating_doc(conn, id, &rating.item_kind, rating.item_id)?);
        }
        for progress in cloud::sync_playback(conn, id)? {
            docs.extend(outbox::playback_doc(conn, id, &progress.item_kind, progress.item_id)?);
        }
        for (kind, key, label, created_at) in cloud::sport_follows(conn, id)? {
            docs.extend(outbox::sport_follow_doc(conn, id, &kind, &key, &label, created_at)?);
        }
        for (channel_id, start_ts, stop_ts, title) in cloud::future_reminders(conn, id, now)? {
            docs.extend(outbox::reminder_doc(conn, id, channel_id, start_ts, stop_ts, &title)?);
        }
        for day in crate::catalog::watch_time::local_days(conn, id, &device)? {
            docs.extend(outbox::watch_day_doc(conn, id, &device, &day)?);
        }
    }
    // Nada de ajustes de cuenta: el único es la clave de TMDB, que no sale nunca a un fichero.
    docs.retain(|doc| !matches!(doc, RemoteDoc::Setting(_)));
    Ok(BackupFile { format: FORMAT.to_string(), version: VERSION, app_version: app_version.to_string(), exported_at: now, docs, ui })
}

/// Mete una copia en este aparato y deja en la cola de la nube lo que cambió.
pub fn import(conn: &Connection, file: BackupFile) -> Result<BackupSummary, BackupError> {
    if file.format != FORMAT {
        return Err(BackupError::NotABackup);
    }
    if file.version > VERSION {
        return Err(BackupError::TooNew);
    }
    let mut docs = file.docs;
    docs.retain(|doc| !matches!(doc, RemoteDoc::Setting(_)));
    adopt_profiles_with_the_same_name(conn, &mut docs)?;

    let before = counts(conn)?;
    let tx = conn.unchecked_transaction()?;
    let mut not_found = 0;
    for doc in &docs {
        if refers_to_missing_item(&tx, doc)? {
            not_found += 1;
            continue;
        }
        super::engine::apply_remote(&tx, doc)?;
    }
    // Que lo importado llegue también a la nube (y a los demás aparatos), si hay cuenta.
    super::engine::enqueue_local_missing_from_cloud(&tx, &[])?;
    tx.commit()?;
    super::engine::wake();
    let after = counts(conn)?;
    Ok(BackupSummary {
        profiles: after.profiles - before.profiles,
        my_list: after.my_list - before.my_list,
        ratings: after.ratings - before.ratings,
        progress: after.progress - before.progress,
        follows: after.follows - before.follows,
        reminders: after.reminders - before.reminders,
        not_found,
    })
}

fn favorites(conn: &Connection, profile_id: i64) -> Result<Vec<(String, i64)>, BackupError> {
    let mut stmt = conn.prepare("SELECT item_kind, item_id FROM favorites WHERE profile_id = ?1")?;
    let rows = stmt.query_map(params![profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Sin cuenta, el mismo perfil tiene otro id en cada aparato: un perfil de la copia que
/// aquí no existe pero se llama igual que uno de aquí se trata como ese (si no, salía
/// «Jorge» dos veces).
fn adopt_profiles_with_the_same_name(conn: &Connection, docs: &mut [RemoteDoc]) -> Result<(), BackupError> {
    let local = cloud::sync_profiles(conn)?;
    let mut rename: HashMap<String, String> = HashMap::new();
    for doc in docs.iter() {
        let RemoteDoc::Profile(p) = doc else { continue };
        if p.deleted || cloud::profile_by_remote_id(conn, &p.id)?.is_some() {
            continue;
        }
        let Some(same) = local.iter().find(|l| l.name.trim().eq_ignore_ascii_case(p.name.trim())) else { continue };
        if let Some(remote_id) = outbox::ensure_profile_remote_id(conn, same.id)? {
            rename.insert(p.id.clone(), remote_id);
        }
    }
    if rename.is_empty() {
        return Ok(());
    }
    for doc in docs.iter_mut() {
        let id = match doc {
            RemoteDoc::Profile(p) => &mut p.id,
            RemoteDoc::Favorite(f) => &mut f.profile_id,
            RemoteDoc::Playback(p) => &mut p.profile_id,
            RemoteDoc::Rating(r) => &mut r.profile_id,
            RemoteDoc::SportFollow(f) => &mut f.profile_id,
            RemoteDoc::Reminder(r) => &mut r.profile_id,
            RemoteDoc::WatchDay(w) => &mut w.profile_id,
            RemoteDoc::Setting(_) => continue,
        };
        if let Some(local_id) = rename.get(id.as_str()) {
            *id = local_id.clone();
        }
    }
    Ok(())
}

fn refers_to_missing_item(conn: &Connection, doc: &RemoteDoc) -> Result<bool, BackupError> {
    let (kind, key) = match doc {
        RemoteDoc::Favorite(f) => (f.item_kind.as_str(), f.item_key.as_str()),
        RemoteDoc::Playback(p) => (p.item_kind.as_str(), p.item_key.as_str()),
        RemoteDoc::Rating(r) => (r.item_kind.as_str(), r.item_key.as_str()),
        RemoteDoc::Reminder(r) => ("channel", r.channel_key.as_str()),
        _ => return Ok(false),
    };
    Ok(reads::item_id_for_key(conn, kind, key)?.is_none())
}

#[derive(Default)]
struct Counts {
    profiles: i64,
    my_list: i64,
    ratings: i64,
    progress: i64,
    follows: i64,
    reminders: i64,
}

fn counts(conn: &Connection) -> Result<Counts, BackupError> {
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
    Ok(Counts {
        profiles: count("SELECT COUNT(*) FROM profiles")?,
        my_list: count("SELECT COUNT(*) FROM favorites")?,
        ratings: count("SELECT COUNT(*) FROM ratings")?,
        progress: count("SELECT COUNT(*) FROM playback_state WHERE item_kind <> 'channel'")?,
        follows: count("SELECT COUNT(*) FROM sport_follows")?,
        reminders: count("SELECT COUNT(*) FROM reminders")?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use crate::db::queries::{self, ProviderKind, TitleFields};
    use crate::db::schema::open_connection_at;
    use crate::sources::model::RawMovie;

    fn seed_movie(conn: &Connection, title: &str) -> i64 {
        let provider_id: i64 = match conn.query_row("SELECT id FROM providers LIMIT 1", [], |r| r.get(0)) {
            Ok(id) => id,
            Err(_) => queries::insert_provider(conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap(),
        };
        let raw = RawMovie {
            remote_id: title.to_string(),
            category_remote_id: None,
            raw_title: title.to_string(),
            container_extension: None,
            logo_url: None,
            added_at: Some(now_epoch()),
            rating: None,
            tmdb_id: None,
        };
        let match_key = title.to_lowercase().replace(' ', "-");
        let fields = TitleFields { clean_title: title, year: None, match_key: &match_key, quality: None, language: None };
        queries::upsert_movie(conn, provider_id, &HashMap::new(), &raw, &fields).unwrap();
        conn.query_row("SELECT id FROM movies WHERE match_key = ?1", params![match_key], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_backup_moves_my_list_ratings_and_progress_to_another_device_without_duplicating_the_profile() {
        let old = open_connection_at(Path::new(":memory:")).unwrap();
        let profile = queries::insert_profile(&old, "Jorge", None, true).unwrap();
        let dune = seed_movie(&old, "Dune");
        let alien = seed_movie(&old, "Alien");
        let solo_aqui = seed_movie(&old, "Solo en el viejo");
        reads::add_favorite(&old, profile, "movie", dune).unwrap();
        reads::add_favorite(&old, profile, "movie", solo_aqui).unwrap();
        reads::set_rating(&old, profile, "movie", alien, 1).unwrap();
        reads::upsert_playback_progress(&old, profile, "movie", alien, 600, Some(7000)).unwrap();
        let file = export(&old, "0.3.1", serde_json::json!({ "Jorge": { "subtitles": "grande" } })).unwrap();
        let text = serde_json::to_string(&file).unwrap();
        assert!(!text.contains("http://x"), "la copia no lleva direcciones del proveedor");

        // El aparato nuevo tiene su propio «Jorge» (otro id) y el mismo catálogo, sin la película que solo estaba en el viejo.
        let new = open_connection_at(Path::new(":memory:")).unwrap();
        let local = queries::insert_profile(&new, "Jorge", None, true).unwrap();
        let dune_here = seed_movie(&new, "Dune");
        let alien_here = seed_movie(&new, "Alien");
        let summary = import(&new, serde_json::from_str(&text).unwrap()).unwrap();

        assert_eq!(summary.profiles, 0, "se usa el «Jorge» que ya había");
        assert_eq!((summary.my_list, summary.ratings, summary.progress, summary.not_found), (1, 1, 1, 1));
        assert!(cloud::favorite_added_at(&new, local, "movie", dune_here).unwrap().is_some());
        let progress = reads::get_playback_progress(&new, local, "movie", alien_here).unwrap().unwrap();
        assert_eq!(progress.position_s, 600);
    }

    #[test]
    fn importing_something_that_is_not_a_backup_says_so() {
        let conn = open_connection_at(Path::new(":memory:")).unwrap();
        let file = BackupFile { format: "otra-cosa".into(), version: 1, app_version: String::new(), exported_at: 0, docs: vec![], ui: serde_json::Value::Null };
        assert!(matches!(import(&conn, file), Err(BackupError::NotABackup)));
    }
}
