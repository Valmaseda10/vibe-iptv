//! Logos de canal (2026-11-02). La mitad de los logos que da el proveedor apuntan a webs
//! caídas o no existen, así que el núcleo busca uno que funcione entre varias fuentes, por
//! este orden, y se queda con el primero que se puede bajar y es una imagen:
//!
//! 1. El elegido a mano para ese canal (`channel_logo_choices`).
//! 2. El del proveedor para ese canal, y el de sus otras versiones (HD, FHD...).
//! 3. El de la guía XMLTV (`<channel><icon>`, ver `catalog::epg`).
//! 4. El del directorio abierto de iptv-org (datos de dominio público), por nombre.
//! 5. El del canal «hermano» sin número («M+ Liga de Campeones 7» → «M+ Liga de Campeones»).
//!
//! Cada imagen se baja una sola vez y se guarda en el aparato (`logos/`); lo que falla no se
//! vuelve a intentar en unos días. Nunca se pide nada al proveedor de streams (R1): los
//! logos están en webs de terceros, y una URL del panel se trata igual, una vez y guardada.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::db::{now_epoch, Result};
use crate::util::redact::redact;

/// Lo que falló no se vuelve a probar hasta pasado esto.
const RETRY_FAILED_AFTER_S: i64 = 7 * 86_400;
/// El directorio de iptv-org se renueva una vez por semana.
const DIRECTORY_MAX_AGE_S: i64 = 7 * 86_400;
const DIRECTORY_AT_KEY: &str = "logo_directory_at";
const CHANNELS_URL: &str = "https://iptv-org.github.io/api/channels.json";
const LOGOS_URL: &str = "https://iptv-org.github.io/api/logos.json";
/// Un logo de más de esto no es un logo.
const MAX_LOGO_BYTES: usize = 1_500_000;
/// Descargas de logos a la vez (una lista de canales pide muchos de golpe).
static DOWNLOADS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(6));
/// Webs que acaban de fallar varias veces: no se esperan sus tiempos de espera para cada logo.
static BAD_HOSTS: LazyLock<Mutex<HashMap<String, (u32, Instant)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
const BAD_HOST_STRIKES: u32 = 3;
const BAD_HOST_FOR: Duration = Duration::from_secs(30 * 60);

/// Nombre de canal normalizado para comparar fuentes: sin tildes, mayúsculas, calidades
/// («HD», «1080p») ni coletillas («por Movistar Plus+»), solo letras y números.
pub fn name_key(name: &str) -> String {
    let lower: String = name.to_lowercase().chars().map(fold_accent).collect();
    let spaced = lower.replace('+', " plus ").replace('&', " and ");
    let words: Vec<&str> = spaced
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !QUALITY_WORDS.contains(w))
        .collect();
    let mut joined = words.join(" ");
    for (from, to) in REWRITES {
        joined = joined.replace(from, to);
    }
    joined.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// La clave sin el número final: «M+ Liga de Campeones 7» → la de «M+ Liga de Campeones».
pub fn base_key(name: &str) -> Option<String> {
    let trimmed = name.trim_end();
    let without = trimmed.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
    (without.len() < trimmed.len() && !without.is_empty()).then(|| name_key(without))
}

const QUALITY_WORDS: &[&str] = &["hd", "fhd", "uhd", "4k", "sd", "hevc", "h265", "1080", "1080p", "720", "720p", "50fps", "backup", "es", "spain", "espana", "tv"];
const REWRITES: &[(&str, &str)] = &[("plus plus", "plus"), ("por movistar plus", ""), ("movistar plus", "movistar"), ("m plus", "movistar")];

fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

/// De dónde sale un logo candidato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogoSource {
    Chosen,
    Provider,
    Guide,
    Directory,
    Sibling,
}

/// Los logos candidatos de un canal, por orden de preferencia y sin repetir.
pub fn candidates(conn: &Connection, channel_id: i64) -> Result<Vec<(LogoSource, String)>> {
    let Some((name, own, epg_id, provider_id)) = conn
        .query_row("SELECT display_name, logo_url, epg_channel_id, provider_id FROM channels WHERE id = ?1", params![channel_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, i64>(3)?))
        })
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let key = name_key(&name);
    let mut out: Vec<(LogoSource, String)> = Vec::new();
    let mut push = |source: LogoSource, url: Option<String>| {
        if let Some(url) = url.filter(|u| u.starts_with("http")) {
            if !out.iter().any(|(_, u)| *u == url) {
                out.push((source, url));
            }
        }
    };

    push(LogoSource::Chosen, conn.query_row("SELECT url FROM channel_logo_choices WHERE name_key = ?1", params![key], |r| r.get(0)).optional()?);
    push(LogoSource::Provider, own);
    let mut siblings = conn.prepare("SELECT DISTINCT logo_url FROM channels WHERE display_name = ?1 AND logo_url IS NOT NULL AND id <> ?2")?;
    for url in siblings.query_map(params![name, channel_id], |r| r.get::<_, String>(0))? {
        push(LogoSource::Provider, Some(url?));
    }
    if let Some(epg_id) = epg_id {
        push(
            LogoSource::Guide,
            conn.query_row("SELECT icon_url FROM epg_channel_icons WHERE channel_id = ?1 ORDER BY (provider_id = ?2) DESC LIMIT 1", params![epg_id, provider_id], |r| r.get(0))
                .optional()?,
        );
    }
    let directory = |k: &str| conn.query_row("SELECT logo_url FROM logo_directory WHERE name_key = ?1", params![k], |r| r.get::<_, String>(0)).optional();
    push(LogoSource::Directory, directory(&key)?);
    if let Some(base) = base_key(&name) {
        push(LogoSource::Directory, directory(&base)?);
        // El canal «hermano» sin número, con su logo del proveedor.
        let mut stmt = conn.prepare("SELECT display_name, logo_url FROM channels WHERE logo_url IS NOT NULL AND display_name LIKE ?1")?;
        let prefix = format!("{}%", name.trim_end().trim_end_matches(|c: char| c.is_ascii_digit()).trim_end());
        for row in stmt.query_map(params![prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (other, url) = row?;
            if name_key(&other) == base {
                push(LogoSource::Sibling, Some(url));
            }
        }
    }
    Ok(out)
}

/// Fichero guardado para una URL de logo, si se sabe que funciona; `Err(())` si falló hace
/// poco (no se reintenta); `Ok(None)` si no se ha probado.
fn known(conn: &Connection, dir: &Path, url: &str, now: i64) -> Result<std::result::Result<Option<PathBuf>, ()>> {
    let row: Option<(bool, Option<String>, i64)> =
        conn.query_row("SELECT ok, file, checked_at FROM logo_checks WHERE url = ?1", params![url], |r| Ok((r.get::<_, i64>(0)? != 0, r.get(1)?, r.get(2)?))).optional()?;
    Ok(match row {
        Some((true, Some(file), _)) if dir.join(&file).exists() => Ok(Some(dir.join(file))),
        Some((false, _, at)) if now - at < RETRY_FAILED_AFTER_S => Err(()),
        _ => Ok(None),
    })
}

fn record(conn: &Connection, url: &str, file: Option<&str>, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO logo_checks (url, ok, file, checked_at) VALUES (?1, ?2, ?3, ?4)",
        params![url, file.is_some() as i64, file, now],
    )?;
    Ok(())
}

/// Qué tipo de imagen es, por sus primeros bytes (no por la cabecera, que miente a veces).
fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
        (head.contains("<svg") || (head.contains("<?xml") && head.contains("svg"))).then_some("svg")
    }
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

fn host_is_bad(host: &str) -> bool {
    BAD_HOSTS.lock().map(|m| m.get(host).is_some_and(|(strikes, at)| *strikes >= BAD_HOST_STRIKES && at.elapsed() < BAD_HOST_FOR)).unwrap_or(false)
}

fn note_host(host: &str, ok: bool) {
    if let Ok(mut m) = BAD_HOSTS.lock() {
        if ok {
            m.remove(host);
        } else {
            let entry = m.entry(host.to_string()).or_insert((0, Instant::now()));
            entry.0 += 1;
            entry.1 = Instant::now();
        }
    }
}

/// Baja una imagen candidata. `None` si no es una imagen que se pueda usar.
async fn download(client: &reqwest::Client, url: &str) -> Option<(Vec<u8>, &'static str)> {
    let host = host_of(url);
    if host_is_bad(&host) {
        return None;
    }
    let _permit = DOWNLOADS.acquire().await.ok()?;
    let result = async {
        let response = client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if bytes.is_empty() || bytes.len() > MAX_LOGO_BYTES {
            return None;
        }
        let kind = image_kind(&bytes)?;
        Some((bytes.to_vec(), kind))
    }
    .await;
    note_host(&host, result.is_some());
    if result.is_none() {
        tracing::debug!(url = %redact(url), "logo de canal que no se puede usar");
    }
    result
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 VIBE-IPTV")
        .build()
        .ok()
}

/// El logo de un canal ya guardado en el aparato, buscándolo si hace falta. `None` si
/// ninguna fuente da uno que funcione (la interfaz enseña las iniciales).
pub async fn resolve(db_path: &Path, dir: &Path, channel_id: i64) -> Option<PathBuf> {
    let db = db_path.to_path_buf();
    let cache = dir.to_path_buf();
    let now = now_epoch();
    let (found, to_try) = tokio::task::spawn_blocking(move || -> Result<(Option<PathBuf>, Vec<String>)> {
        let conn = crate::db::schema::open_connection_at(&db)?;
        let mut to_try = Vec::new();
        for (_, url) in candidates(&conn, channel_id)? {
            match known(&conn, &cache, &url, now)? {
                // El primero que funciona gana, pero solo si no hay uno preferido sin probar antes.
                Ok(Some(path)) if to_try.is_empty() => return Ok((Some(path), Vec::new())),
                Ok(Some(_)) => break,
                Ok(None) => to_try.push(url),
                Err(()) => {}
            }
        }
        Ok((None, to_try))
    })
    .await
    .ok()?
    .ok()?;
    if found.is_some() {
        return found;
    }
    let client = client()?;
    for url in to_try {
        let downloaded = download(&client, &url).await;
        let file = match &downloaded {
            Some((bytes, kind)) => {
                let name = format!("{}.{kind}", Sha256::digest(url.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect::<String>());
                let saved = tokio::fs::create_dir_all(dir).await.is_ok() && tokio::fs::write(dir.join(&name), bytes).await.is_ok();
                saved.then_some(name)
            }
            None => None,
        };
        let db = db_path.to_path_buf();
        let (url_for_db, file_for_db) = (url.clone(), file.clone());
        let _ = tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = crate::db::schema::open_connection_at(&db)?;
            record(&conn, &url_for_db, file_for_db.as_deref(), now)
        })
        .await;
        if let Some(name) = file {
            return Some(dir.join(name));
        }
    }
    // Ninguno nuevo funcionó: el primero que ya se sabía bueno, si lo hay.
    let db = db_path.to_path_buf();
    let cache = dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Option<PathBuf>> {
        let conn = crate::db::schema::open_connection_at(&db)?;
        for (_, url) in candidates(&conn, channel_id)? {
            if let Ok(Some(path)) = known(&conn, &cache, &url, now)? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    })
    .await
    .ok()?
    .ok()?
}

// ─────────────── Directorio de iptv-org ───────────────

#[derive(Deserialize)]
struct DirectoryChannel {
    id: String,
    name: String,
    #[serde(default)]
    alt_names: Vec<String>,
    country: Option<String>,
}

#[derive(Deserialize)]
struct DirectoryLogo {
    channel: String,
    #[serde(default)]
    feed: Option<String>,
    #[serde(default = "yes")]
    in_use: bool,
    url: String,
    #[serde(default)]
    width: Option<f64>,
}

fn yes() -> bool {
    true
}

/// Del JSON de iptv-org a «nombre normalizado → logo». Gana el canal español si dos se
/// llaman igual, y de cada canal el logo en uso del canal principal (no de una variante
/// regional) más ancho.
pub fn build_directory(channels_json: &str, logos_json: &str) -> serde_json::Result<Vec<(String, String, String, Option<String>)>> {
    let channels: Vec<DirectoryChannel> = serde_json::from_str(channels_json)?;
    let logos: Vec<DirectoryLogo> = serde_json::from_str(logos_json)?;
    let mut best: HashMap<&str, (&str, f64)> = HashMap::new();
    for logo in logos.iter().filter(|l| l.in_use && l.url.starts_with("http")) {
        let score = logo.width.unwrap_or(100.0).min(1000.0) + if logo.feed.is_none() { 2000.0 } else { 0.0 };
        let entry = best.entry(logo.channel.as_str()).or_insert((logo.url.as_str(), -1.0));
        if score > entry.1 {
            *entry = (logo.url.as_str(), score);
        }
    }
    let mut ordered: Vec<&DirectoryChannel> = channels.iter().filter(|c| best.contains_key(c.id.as_str())).collect();
    ordered.sort_by_key(|c| if c.country.as_deref() == Some("ES") { 0 } else { 1 });
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for channel in ordered {
        let Some((url, _)) = best.get(channel.id.as_str()) else { continue };
        for name in std::iter::once(&channel.name).chain(channel.alt_names.iter()) {
            let key = name_key(name);
            if key.len() >= 2 && seen.insert(key.clone()) {
                out.push((key, (*url).to_string(), channel.id.clone(), channel.country.clone()));
            }
        }
    }
    Ok(out)
}

/// Renueva el directorio de iptv-org si tiene más de una semana. Sin red no pasa nada: se
/// queda el que hubiera.
pub async fn refresh_directory(db_path: &Path) {
    let db = db_path.to_path_buf();
    let fresh = tokio::task::spawn_blocking(move || -> Result<bool> {
        let conn = crate::db::schema::open_connection_at(&db)?;
        let at: i64 = crate::db::reads::get_setting(&conn, DIRECTORY_AT_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        Ok(now_epoch() - at < DIRECTORY_MAX_AGE_S)
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or(true);
    if fresh {
        return;
    }
    let Some(client) = reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("VIBE-IPTV").build().ok() else { return };
    let fetch = |url: &'static str| {
        let client = client.clone();
        async move { client.get(url).send().await.ok()?.error_for_status().ok()?.text().await.ok() }
    };
    let (Some(channels), Some(logos)) = tokio::join!(fetch(CHANNELS_URL), fetch(LOGOS_URL)) else {
        tracing::info!("no se pudo renovar el directorio de logos (iptv-org)");
        return;
    };
    let db = db_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<usize, String> {
        let entries = build_directory(&channels, &logos).map_err(|e| e.to_string())?;
        let conn = crate::db::schema::open_connection_at(&db).map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM logo_directory", []).map_err(|e| e.to_string())?;
        {
            let mut insert = tx.prepare("INSERT OR IGNORE INTO logo_directory (name_key, logo_url, source_id, country) VALUES (?1, ?2, ?3, ?4)").map_err(|e| e.to_string())?;
            for (key, url, id, country) in &entries {
                insert.execute(params![key, url, id, country]).map_err(|e| e.to_string())?;
            }
        }
        crate::db::queries::set_setting(&tx, DIRECTORY_AT_KEY, &now_epoch().to_string()).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(entries.len())
    })
    .await;
    match result {
        Ok(Ok(count)) => tracing::info!(count, "directorio de logos renovado (iptv-org)"),
        Ok(Err(err)) => tracing::warn!(error = %err, "directorio de logos sin renovar"),
        Err(err) => tracing::warn!(error = %err, "directorio de logos sin renovar"),
    }
}

/// Cuántos canales (por nombre) tienen ya un logo que funciona, de cuántos.
pub fn coverage(conn: &Connection, dir: &Path) -> Result<(i64, i64)> {
    let now = now_epoch();
    let mut stmt = conn.prepare("SELECT MIN(id) FROM channels GROUP BY display_name")?;
    let ids: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut with_logo = 0;
    for id in &ids {
        for (_, url) in candidates(conn, *id)? {
            if let Ok(Some(_)) = known(conn, dir, &url, now)? {
                with_logo += 1;
                break;
            }
        }
    }
    Ok((with_logo, ids.len() as i64))
}

/// Busca el logo de todos los canales (uno por nombre), poco a poco. Para «Buscar logos» en
/// Ajustes y tras sincronizar.
pub async fn resolve_all(db_path: &Path, dir: &Path) -> (i64, i64) {
    refresh_directory(db_path).await;
    let db = db_path.to_path_buf();
    let ids: Vec<i64> = tokio::task::spawn_blocking(move || -> Result<Vec<i64>> {
        let conn = crate::db::schema::open_connection_at(&db)?;
        let mut stmt = conn.prepare("SELECT MIN(id) FROM channels GROUP BY display_name")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();
    let total = ids.len() as i64;
    let mut found = 0;
    for chunk in ids.chunks(12) {
        let results = futures_util::future::join_all(chunk.iter().map(|id| resolve(db_path, dir, *id))).await;
        found += results.iter().filter(|r| r.is_some()).count() as i64;
    }
    (found, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_compared_without_accents_qualities_or_movistar_suffixes() {
        assert_eq!(name_key("La 1 HD"), name_key("LA 1"));
        assert_eq!(name_key("Clásicos por Movistar Plus+"), name_key("Clasicos"));
        assert_eq!(name_key("M+ Deportes 2 FHD"), name_key("Movistar Deportes 2"));
        assert_ne!(name_key("La 1"), name_key("La 2"));
        assert_eq!(base_key("M+ Liga de Campeones 7"), Some(name_key("M+ Liga de Campeones")));
        assert_eq!(base_key("Antena 3 HD"), None);
    }

    #[test]
    fn recognises_image_files_by_their_first_bytes() {
        assert_eq!(image_kind(&[0x89, b'P', b'N', b'G', 0, 0]), Some("png"));
        assert_eq!(image_kind(b"<svg xmlns='http://www.w3.org/2000/svg'></svg>"), Some("svg"));
        assert_eq!(image_kind(b"<html>404</html>"), None);
    }

    #[test]
    fn the_directory_prefers_spanish_channels_and_the_main_logo() {
        let channels = r#"[
            {"id":"La1.es","name":"La 1","alt_names":["TVE 1"],"country":"ES"},
            {"id":"La1.mx","name":"La 1","alt_names":[],"country":"MX"},
            {"id":"SinLogo.es","name":"Sin logo","alt_names":[],"country":"ES"}
        ]"#;
        let logos = r#"[
            {"channel":"La1.es","feed":"Canarias","in_use":true,"url":"https://x/regional.png","width":900},
            {"channel":"La1.es","feed":null,"in_use":true,"url":"https://x/la1.png","width":300},
            {"channel":"La1.mx","feed":null,"in_use":true,"url":"https://x/mexico.png","width":512}
        ]"#;
        let entries = build_directory(channels, logos).unwrap();
        let find = |name: &str| entries.iter().find(|e| e.0 == name_key(name)).map(|e| e.1.as_str());
        assert_eq!(find("La 1"), Some("https://x/la1.png"));
        assert_eq!(find("TVE 1"), Some("https://x/la1.png"));
        assert_eq!(find("Sin logo"), None);
    }

    #[test]
    fn candidates_go_from_the_chosen_one_to_the_sibling_without_number() {
        let conn = crate::db::schema::open_connection_at(Path::new(":memory:")).unwrap();
        let provider = crate::db::queries::insert_provider(&conn, crate::db::queries::ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let add = |remote: &str, name: &str, logo: Option<&str>, epg: Option<&str>| {
            conn.execute(
                "INSERT INTO channels (provider_id, remote_id, raw_name, display_name, logo_url, epg_channel_id) VALUES (?1, ?2, ?3, ?3, ?4, ?5)",
                params![provider, remote, name, logo, epg],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let seven = add("7", "M+ Liga de Campeones 7", Some("https://muerto/7.png"), Some("lc7.es"));
        add("1", "M+ Liga de Campeones", Some("https://bien/lc.png"), None);
        conn.execute("INSERT INTO epg_channel_icons (provider_id, channel_id, icon_url) VALUES (?1, 'lc7.es', 'https://guia/lc7.png')", params![provider]).unwrap();
        conn.execute("INSERT INTO logo_directory (name_key, logo_url, source_id) VALUES (?1, 'https://iptv-org/lc.png', 'x')", params![name_key("M+ Liga de Campeones 7")]).unwrap();

        let list = candidates(&conn, seven).unwrap();
        let order: Vec<LogoSource> = list.iter().map(|(s, _)| *s).collect();
        assert_eq!(order, vec![LogoSource::Provider, LogoSource::Guide, LogoSource::Directory, LogoSource::Sibling]);

        conn.execute("INSERT INTO channel_logo_choices (name_key, url, updated_at) VALUES (?1, 'https://elegido/lc7.png', 0)", params![name_key("M+ Liga de Campeones 7")]).unwrap();
        assert_eq!(candidates(&conn, seven).unwrap()[0], (LogoSource::Chosen, "https://elegido/lc7.png".to_string()));
    }
}
