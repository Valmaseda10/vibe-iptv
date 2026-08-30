//! Parser de listas M3U en streaming (línea a línea, nunca `read_to_string` del fichero
//! completo). Ver `docs/ARCHITECTURE.md`, sección "Capa de fuentes", y la sección M3U de
//! `docs/XTREAM-API.md`.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::BufRead;
use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use reqwest::Url;

use super::error::{Result, SourceError};
use super::model::{
    AccountInfo, MediaKind, RawCategory, RawChannel, RawEpisode, RawMovie, RawSeries,
    RawSeriesDetail, StreamRef, XmltvStream,
};
use super::Source;

static ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([A-Za-z][A-Za-z0-9_-]*)="([^"]*)""#).unwrap());
static TVG_URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)x-tvg-url="([^"]*)""#).unwrap());

// Patrones de episodio, en orden de prueba. Extracción cruda para poder agrupar
// episodios de M3U (docs/DATA-MODEL.md, regla 5 de normalización); la limpieza fina de
// títulos es de la fase F3, no de aquí.
static EPISODE_PATTERNS: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        Regex::new(r"(?i)s(\d{1,2})e(\d{1,3})").unwrap(),
        Regex::new(r"(?i)\b(\d{1,2})x(\d{1,3})\b").unwrap(),
        Regex::new(r"(?i)\bT(\d{1,2})\s*E(\d{1,3})\b").unwrap(),
        Regex::new(r"(?i)Temporada\s*(\d{1,2}).{0,15}?Cap[ií]tulo\s*(\d{1,3})").unwrap(),
    ]
});

pub struct M3uSource {
    channels: Vec<RawChannel>,
    movies: Vec<RawMovie>,
    series: Vec<RawSeries>,
    series_episodes: HashMap<String, Vec<RawEpisode>>,
    categories: HashMap<MediaKind, HashMap<String, String>>,
    /// URL real (con credenciales si las hay) indexada por el `remote_id` redactado que
    /// se expone en los tipos `Raw*`. Nunca se serializa ni se registra tal cual.
    urls_by_id: HashMap<String, String>,
    epg_url: Option<String>,
}

impl M3uSource {
    pub fn parse<R: BufRead>(reader: R) -> Result<Self> {
        let mut channels = Vec::new();
        let mut movies = Vec::new();
        let mut series_map: HashMap<String, RawSeries> = HashMap::new();
        let mut series_episodes: HashMap<String, Vec<RawEpisode>> = HashMap::new();
        let mut categories: HashMap<MediaKind, HashMap<String, String>> = HashMap::new();
        let mut urls_by_id: HashMap<String, String> = HashMap::new();
        let mut epg_url = None;
        let mut pending: Option<ExtInfEntry> = None;

        for line in reader.lines() {
            let line = line.map_err(|err| SourceError::M3uRead(err.to_string()))?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(rest) = line.strip_prefix("#EXTM3U") {
                if let Some(caps) = TVG_URL_RE.captures(rest) {
                    epg_url = non_empty(caps[1].to_string());
                }
                continue;
            }

            if let Some(rest) = line.strip_prefix("#EXTINF:") {
                pending = Some(parse_extinf(rest));
                continue;
            }

            if line.starts_with('#') {
                continue; // #EXTVLCOPT, #EXTGRP, etc.: sin uso en F1.
            }

            let Some(entry) = pending.take() else {
                continue; // URL sin #EXTINF previo: línea descartada.
            };

            let url = line.to_string();
            let kind = classify(&url, entry.group_title.as_deref());
            let remote_id = opaque_id(&url);
            let extension = extension_from_url(&url);
            urls_by_id.insert(remote_id.clone(), url);

            let category_remote_id = entry.group_title.as_ref().map(|group| {
                let slug_value = slug(group);
                categories
                    .entry(kind)
                    .or_default()
                    .entry(group.clone())
                    .or_insert(slug_value)
                    .clone()
            });

            match kind {
                MediaKind::Live => channels.push(RawChannel {
                    remote_id,
                    category_remote_id,
                    raw_name: entry.display_name,
                    logo_url: entry.tvg_logo,
                    epg_channel_id: entry.tvg_id,
                    has_catchup: false,
                    catchup_days: 0,
                    sort_order: None,
                }),
                MediaKind::Movie => movies.push(RawMovie {
                    remote_id,
                    category_remote_id,
                    raw_title: entry.display_name,
                    container_extension: extension,
                    logo_url: entry.tvg_logo,
                    added_at: None,
                    rating: None,
                    tmdb_id: None,
                }),
                MediaKind::Series => {
                    let (title_prefix, season, number) =
                        extract_episode(&entry.display_name).unwrap_or_else(|| (entry.display_name.clone(), 1, 1));
                    let key = format!("m3u-series:{}", slug(&title_prefix));

                    series_map.entry(key.clone()).or_insert_with(|| RawSeries {
                        remote_id: key.clone(),
                        raw_title: title_prefix,
                        cover_url: entry.tvg_logo.clone(),
                        plot: None,
                        release_date: None,
                    });

                    series_episodes.entry(key).or_default().push(RawEpisode {
                        remote_id,
                        season,
                        number,
                        title: Some(entry.display_name),
                        container_extension: extension,
                        duration_s: None,
                        still_url: entry.tvg_logo,
                    });
                }
            }
        }

        Ok(Self {
            channels,
            movies,
            series: series_map.into_values().collect(),
            series_episodes,
            categories,
            urls_by_id,
            epg_url,
        })
    }
}

#[async_trait]
impl Source for M3uSource {
    async fn account_info(&self) -> Result<AccountInfo> {
        // Una lista M3U no expone cuenta ni límite de conexiones; se asume 1 hasta que
        // el usuario la convierta a Xtream (ver detect_embedded_xtream_credentials).
        Ok(AccountInfo {
            auth_ok: true,
            status: "unknown".to_string(),
            expires_at: None,
            max_connections: 1,
            active_connections: None,
        })
    }

    async fn categories(&self, kind: MediaKind) -> Result<Vec<RawCategory>> {
        Ok(self
            .categories
            .get(&kind)
            .map(|by_name| {
                by_name
                    .iter()
                    .map(|(name, remote_id)| RawCategory {
                        remote_id: remote_id.clone(),
                        kind,
                        name: name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn live_channels(&self) -> Result<Vec<RawChannel>> {
        Ok(self.channels.clone())
    }

    async fn movies(&self) -> Result<Vec<RawMovie>> {
        Ok(self.movies.clone())
    }

    async fn series(&self) -> Result<Vec<RawSeries>> {
        Ok(self.series.clone())
    }

    async fn series_detail(&self, id: &str) -> Result<RawSeriesDetail> {
        self.series_episodes
            .get(id)
            .cloned()
            .map(|episodes| RawSeriesDetail { episodes })
            .ok_or_else(|| SourceError::NotFound { remote_id: id.to_string() })
    }

    async fn epg_xmltv(&self) -> Result<Option<XmltvStream>> {
        Ok(self.epg_url.clone().map(|url| XmltvStream { url }))
    }

    fn stream_url(&self, item: &StreamRef) -> Result<Url> {
        let raw = self
            .urls_by_id
            .get(&item.remote_id)
            .ok_or_else(|| SourceError::NotFound { remote_id: item.remote_id.clone() })?;
        Url::parse(raw).map_err(|_| SourceError::invalid_response(raw, "URL de stream M3U inválida"))
    }
}

struct ExtInfEntry {
    tvg_id: Option<String>,
    tvg_logo: Option<String>,
    group_title: Option<String>,
    display_name: String,
}

/// El nombre visible es lo que va después de la **última** coma de la línea `#EXTINF`,
/// no `tvg-name` (ver `docs/XTREAM-API.md`).
fn parse_extinf(rest: &str) -> ExtInfEntry {
    let (attrs_part, display_name) = match rest.rfind(',') {
        Some(idx) => (&rest[..idx], rest[idx + 1..].trim().to_string()),
        None => (rest, String::new()),
    };

    let mut tvg_id = None;
    let mut tvg_logo = None;
    let mut group_title = None;
    for caps in ATTR_RE.captures_iter(attrs_part) {
        let value = non_empty(caps[2].to_string());
        match caps[1].to_ascii_lowercase().as_str() {
            "tvg-id" => tvg_id = value,
            "tvg-logo" => tvg_logo = value,
            "group-title" => group_title = value,
            _ => {}
        }
    }

    ExtInfEntry { tvg_id, tvg_logo, group_title, display_name }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Clasifica por la ruta de la URL (`/live/`, `/movie/`, `/series/`) y, si no está, por
/// heurística sobre `group-title`. Por defecto, directo: es el caso mayoritario en M3U.
fn classify(url: &str, group_title: Option<&str>) -> MediaKind {
    let lower_url = url.to_ascii_lowercase();
    if lower_url.contains("/live/") {
        return MediaKind::Live;
    }
    if lower_url.contains("/movie/") {
        return MediaKind::Movie;
    }
    if lower_url.contains("/series/") {
        return MediaKind::Series;
    }

    if let Some(group) = group_title {
        let normalized = group.to_lowercase();
        let is_movie = ["vod", "pelicula", "película", "movie"]
            .iter()
            .any(|kw| normalized.contains(kw));
        if is_movie {
            return MediaKind::Movie;
        }
        if normalized.contains("serie") {
            return MediaKind::Series;
        }
    }

    MediaKind::Live
}

/// Busca un patrón de episodio (`S01E02`, `1x02`, `T1 E2`, `Temporada 1 Capitulo 2`) y
/// devuelve el título sin el patrón, la temporada y el número. Si no hay patrón
/// reconocible, el propio título se trata como episodio único de una temporada 1.
fn extract_episode(title: &str) -> Option<(String, u32, u32)> {
    for pattern in EPISODE_PATTERNS.iter() {
        let Some(caps) = pattern.captures(title) else { continue };
        let season: u32 = caps[1].parse().ok()?;
        let number: u32 = caps[2].parse().ok()?;
        let whole = caps.get(0).unwrap();
        let prefix = title[..whole.start()]
            .trim_end_matches(|c: char| c == '-' || c == '|' || c == ':' || c.is_whitespace())
            .trim();
        let prefix = if prefix.is_empty() { title.to_string() } else { prefix.to_string() };
        return Some((prefix, season, number));
    }
    None
}

/// Identificador estable para una URL de M3U que nunca puede contener las credenciales
/// embebidas en ella (regla R5). No basta con `util::redact()` aquí: esa función solo
/// reconoce la forma `/live|movie|series/usuario/contraseña/id` de Xtream, pero un M3U
/// puede traer credenciales en cualquier posición de la ruta bajo un segmento distinto
/// (`/vod/`, `/stream/`, lo que sea). Un hash del contenido nunca puede filtrar un
/// substring del original, sea cual sea su forma.
fn opaque_id(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("m3u:{:016x}", hasher.finish())
}

fn extension_from_url(url: &str) -> Option<String> {
    let last_segment = url.rsplit('/').next()?;
    let last_segment = last_segment.split(['?', '#']).next().unwrap_or(last_segment);
    let (_, ext) = last_segment.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 5 {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

/// Clave interna de agrupación de M3U: ascii, minúsculas, sin diacríticos eliminados
/// (eso lo hace `catalog::normalize` en F3). Suficiente para no duplicar series dentro
/// de una misma lista.
fn slug(input: &str) -> String {
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

/// Credenciales Xtream detectadas en una URL `get.php`. Ver `docs/XTREAM-API.md`,
/// sección "Detección de credenciales embebidas". Solo detecta el patrón; probar
/// `player_api.php` y proponer la conversión al usuario es una pantalla de F2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XtreamCredentials {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

pub fn detect_embedded_xtream_credentials(url: &str) -> Option<XtreamCredentials> {
    let parsed = Url::parse(url).ok()?;
    if !parsed.path().ends_with("/get.php") {
        return None;
    }

    let mut username = None;
    let mut password = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "username" => username = Some(value.into_owned()),
            "password" => password = Some(value.into_owned()),
            _ => {}
        }
    }

    let mut base = parsed.clone();
    base.set_path("");
    base.set_query(None);

    Some(XtreamCredentials {
        base_url: base.to_string().trim_end_matches('/').to_string(),
        username: username?,
        password: password?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parsed_fixture() -> M3uSource {
        let path = format!("{}/tests/fixtures/m3u/sample.m3u", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("no se pudo leer {path}: {err}"));
        M3uSource::parse(Cursor::new(text.into_bytes())).unwrap()
    }

    #[tokio::test]
    async fn parses_fixture_into_expected_counts_per_kind() {
        let source = parsed_fixture();

        let channels = source.live_channels().await.unwrap();
        let movies = source.movies().await.unwrap();
        let series = source.series().await.unwrap();

        // 50 entradas en el fixture: 43 directo, 2 películas, 4 series distintas (5
        // episodios agrupados en 4 títulos, ver comentario en el fixture de prueba).
        assert_eq!(channels.len(), 43);
        assert_eq!(movies.len(), 2);
        assert_eq!(series.len(), 4);
    }

    #[tokio::test]
    async fn display_name_is_text_after_last_comma_not_tvg_name() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();

        let canal_uno = channels.iter().find(|c| c.epg_channel_id.as_deref() == Some("canaluno.es")).unwrap();
        assert_eq!(canal_uno.raw_name, "ES | Canal Uno HD");
    }

    #[tokio::test]
    async fn trims_surrounding_whitespace_but_keeps_internal_spacing() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();

        let raro = channels.iter().find(|c| c.raw_name.contains("Raro")).unwrap();
        assert_eq!(raro.raw_name, "ES  |   Canal   Raro");
    }

    #[tokio::test]
    async fn extinf_followed_by_extvlcopt_still_captures_the_url() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();
        assert!(channels.iter().any(|c| c.raw_name == "Canal Tres"));
    }

    #[tokio::test]
    async fn classifies_by_path_when_present() {
        let source = parsed_fixture();
        let movies = source.movies().await.unwrap();
        assert!(movies.iter().any(|m| m.raw_title.starts_with("Película de Estreno")));
    }

    #[tokio::test]
    async fn classifies_by_group_title_heuristic_when_path_does_not_say() {
        let source = parsed_fixture();
        let movies = source.movies().await.unwrap();
        // Esta entrada usa /vod/ en la URL: solo el group-title "PELICULAS ACCION" dice
        // que es película.
        assert!(movies.iter().any(|m| m.raw_title == "Otra Película Sin Ruta Movie"));
    }

    #[tokio::test]
    async fn entry_without_path_hint_or_group_title_defaults_to_live() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();
        assert!(channels.iter().any(|c| c.raw_name == "Canal Comodín Sin Grupo"));
    }

    #[tokio::test]
    async fn groups_episodes_from_sxxeyy_and_nxnn_patterns_under_the_same_series() {
        let source = parsed_fixture();
        let series = source.series().await.unwrap();
        let serie_ejemplo = series.iter().find(|s| s.raw_title == "Serie de Ejemplo").unwrap();

        let detail = source.series_detail(&serie_ejemplo.remote_id).await.unwrap();
        assert_eq!(detail.episodes.len(), 2);
        assert!(detail.episodes.iter().any(|e| e.season == 1 && e.number == 2));
        assert!(detail.episodes.iter().any(|e| e.season == 1 && e.number == 3));
    }

    #[tokio::test]
    async fn recognizes_txey_and_temporada_capitulo_patterns_as_distinct_series() {
        let source = parsed_fixture();
        let series = source.series().await.unwrap();

        let otra_serie = series.iter().find(|s| s.raw_title == "Otra Serie").unwrap();
        let detail = source.series_detail(&otra_serie.remote_id).await.unwrap();
        assert_eq!(detail.episodes[0].season, 2);
        assert_eq!(detail.episodes[0].number, 5);

        let otra_serie_mas = series.iter().find(|s| s.raw_title == "Otra Serie Mas").unwrap();
        let detail = source.series_detail(&otra_serie_mas.remote_id).await.unwrap();
        assert_eq!(detail.episodes[0].season, 3);
        assert_eq!(detail.episodes[0].number, 1);
    }

    #[tokio::test]
    async fn series_entry_without_episode_pattern_becomes_its_own_singleton() {
        let source = parsed_fixture();
        let series = source.series().await.unwrap();

        let singleton = series
            .iter()
            .find(|s| s.raw_title == "Documental Especial Sin Patron De Episodio")
            .unwrap();
        let detail = source.series_detail(&singleton.remote_id).await.unwrap();
        assert_eq!(detail.episodes.len(), 1);
        assert_eq!(detail.episodes[0].season, 1);
        assert_eq!(detail.episodes[0].number, 1);
    }

    #[tokio::test]
    async fn x_tvg_url_is_captured_from_the_header_line() {
        let source = parsed_fixture();
        let epg = source.epg_xmltv().await.unwrap().unwrap();
        assert_eq!(epg.url, "http://panel.example.com/xmltv.php?username=demo&password=demo");
    }

    #[tokio::test]
    async fn group_titles_become_synthetic_categories_per_kind() {
        let source = parsed_fixture();

        let live_categories = source.categories(MediaKind::Live).await.unwrap();
        assert_eq!(live_categories.len(), 2);
        assert!(live_categories.iter().any(|c| c.name == "ESPAÑA | GENERAL"));
        assert!(live_categories.iter().any(|c| c.name == "DOCUMENTALES"));

        let movie_categories = source.categories(MediaKind::Movie).await.unwrap();
        assert_eq!(movie_categories.len(), 2);

        let series_categories = source.categories(MediaKind::Series).await.unwrap();
        assert_eq!(series_categories.len(), 1);
        assert_eq!(series_categories[0].name, "SERIES ES");
    }

    #[tokio::test]
    async fn remote_ids_never_contain_the_embedded_credentials() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();
        let movies = source.movies().await.unwrap();

        assert!(channels.iter().all(|c| !c.remote_id.contains("demo")));
        assert!(movies.iter().all(|m| !m.remote_id.contains("demo")));
    }

    #[tokio::test]
    async fn stream_url_resolves_back_to_the_original_url_with_credentials() {
        let source = parsed_fixture();
        let channels = source.live_channels().await.unwrap();
        let canal_dos = channels.iter().find(|c| c.raw_name == "Canal Dos").unwrap();

        let item = StreamRef {
            kind: MediaKind::Live,
            remote_id: canal_dos.remote_id.clone(),
            container_extension: None,
        };
        let url = source.stream_url(&item).unwrap();
        assert_eq!(url.as_str(), "http://panel.example.com:8080/live/demo/demo/2.ts");
    }

    #[test]
    fn detects_get_php_url_as_embedded_xtream_credentials() {
        let creds = detect_embedded_xtream_credentials(
            "http://panel.example.com:8080/get.php?username=juan&password=hunter2&type=m3u_plus&output=ts",
        )
        .unwrap();
        assert_eq!(creds.base_url, "http://panel.example.com:8080");
        assert_eq!(creds.username, "juan");
        assert_eq!(creds.password, "hunter2");
    }

    #[test]
    fn does_not_detect_credentials_on_a_normal_m3u_url() {
        assert!(detect_embedded_xtream_credentials("http://panel.example.com:8080/playlist.m3u").is_none());
        assert!(detect_embedded_xtream_credentials("http://panel.example.com:8080/live/user/pass/1.ts").is_none());
    }
}
