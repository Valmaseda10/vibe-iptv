//! Normalización de títulos y `match_key`. Seis reglas en orden, ver la sección
//! "Normalización de títulos" de `docs/DATA-MODEL.md`.

use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedTitle {
    pub clean_title: String,
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub language: Option<String>,
}

const LANGUAGE_CODES: &[&str] = &[
    "ES", "EN", "LAT", "LATAM", "LATINO", "PT", "FR", "IT", "DE", "MX", "AR", "US", "UK", "VO", "VOSE",
];
const QUALITY_MARKERS: &[&str] = &["UHD", "FHD", "4K", "1080P", "720P", "HDR", "DOLBY", "HD", "SD"];
const NOISE_PREFIXES: &[&str] = &["VIP"];
const RELEASE_TAGS: &[&str] = &[
    "WEB-DL", "BLURAY", "HDTV", "X264", "X265", "HEVC", "DUAL", "CASTELLANO", "SUBTITULADO", "MULTI", "AC3", "DTS",
];
const ARTICLES: &[&str] = &["el", "la", "los", "las", "un", "una", "unos", "unas", "the", "a", "an"];

// Dos formas: entre corchetes (el propio corchete ya delimita, no hace falta separador
// detrás) o suelto con un separador explícito (sin él, "ES" podría ser parte del título).
// El separador se captura (grupo 3): una barra vertical tras una etiqueta desconocida
// en mayúsculas es una etiqueta de categoría del panel, nunca parte del título.
static PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:\[\s*([A-Za-zÀ-ÿ0-9]{1,6})\s*\]|([A-Za-zÀ-ÿ0-9]{1,6})\s*([|:\-]))\s*").unwrap()
});
/// Cuántas etiquetas de prefijo seguidas se retiran como mucho ("ES| DEP| Fútbol").
const MAX_PREFIX_TAGS: usize = 3;
static HASH_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*#\s*").unwrap());
// Solo formas entre paréntesis/corchetes: un año suelto al final es indistinguible de
// parte del título ("1917", "2012", "Blade Runner 2049"). Ver docs/DECISIONS.md.
static YEAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(\[]\s*(\d{4})\s*[)\]]").unwrap());
/// Única excepción al año suelto: al final y tras un guion ("Beethoven 2 - 1993", visto en
/// la lista real del usuario). El guion lo separa del título sin ambigüedad.
static DASH_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s[-–]\s*(\d{4})[\s._]*$").unwrap());
/// "1080 p", con espacio: la forma pegada ya la reconoce `QUALITY_MARKERS`.
static SPACED_QUALITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(2160|1080|720|480)(\s+p\b|\s*$)").unwrap());
static BRACKETED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]*)\]").unwrap());
/// Idioma al final del título ("... Latino", "... VOSE"). Solo al final: en medio puede
/// ser parte del título ("Latino Americano").
static TRAILING_LANGUAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\s_.\-(]+(latino|vose|vo)[\s.)]*$").unwrap());
/// Marcas de feed que algunos paneles ponen al final del nombre del canal ("DAZN LaLiga
/// HD@", "M+ LaLiga TV HD ?"): una "@" en cualquier caso, otros signos solo sueltos.
static FEED_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\s*@+|\s+[?*#]+)\s*$").unwrap());
/// Palabras de un corchete técnico ("[MicroHD 1080p]", "[WEB-DL m1080p]", "[ES]").
const TECHNICAL_BRACKET_WORDS: &[&str] = &["MICROHD", "BDRIP", "WEBRIP", "WEB", "DL", "SUBS", "SUB", "VOS", "VOSE", "M1080P", "M720P"];
/// Lo que queda de "(1080p-x265)" tras retirar calidad y etiquetas: "(-)".
static EMPTY_BRACKETS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(\[][\s\-_.,|:]*[)\]]").unwrap());
static EPISODE_PATTERNS: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        // `\s*` entre número y "E": un proveedor real usa "S4 E5" con espacio, no solo
        // "S04E05" pegado (ver docs/DECISIONS.md, F8) — el resto de reglas ya toleraban
        // el espacio (T1 E2), esta se había quedado más estricta sin motivo.
        Regex::new(r"(?i)\bs\d{1,2}\s*e\d{1,3}\b").unwrap(),
        Regex::new(r"(?i)\b\d{1,2}x\d{1,3}\b").unwrap(),
        Regex::new(r"(?i)\bT\d{1,2}\s*E\d{1,3}\b").unwrap(),
        Regex::new(r"(?i)Temporada\s*\d{1,2}.{0,15}?Cap[ií]tulo\s*\d{1,3}").unwrap(),
    ]
});

/// Aplica las seis reglas en orden y devuelve el título limpio más los metadatos
/// extraídos. Nunca falla ni entra en pánico: en el peor caso, `clean_title` es el
/// original solo con espacios sobrantes recortados.
pub fn normalize(raw_title: &str) -> NormalizedTitle {
    let mut text = raw_title.trim().to_string();
    let mut language = None;
    let mut quality = None;

    // Regla 1: prefijos de idioma o país (y ruido tipo VIP/#, que se descarta).
    if let Some(rest) = HASH_PREFIX_RE.find(&text).map(|m| text[m.end()..].to_string()) {
        text = rest;
    }
    for _ in 0..MAX_PREFIX_TAGS {
        let Some(caps) = PREFIX_RE.captures(&text) else { break };
        // La alternativa entre corchetes captura en el grupo 1, la suelta con separador
        // en el grupo 2; solo una de las dos existe según cuál coincidió.
        let Some(tag_match) = caps.get(1).or_else(|| caps.get(2)) else { break };
        let raw_tag = tag_match.as_str();
        let tag = raw_tag.to_uppercase();
        let whole_end = caps.get(0).map(|m| m.end()).unwrap_or(0);
        let pipe_separated = caps.get(3).is_some_and(|sep| sep.as_str() == "|");
        if LANGUAGE_CODES.contains(&tag.as_str()) {
            language.get_or_insert(tag);
        } else if QUALITY_MARKERS.contains(&tag.as_str()) {
            quality.get_or_insert(tag);
        } else if NOISE_PREFIXES.contains(&tag.as_str()) || (pipe_separated && raw_tag == tag) {
            // Etiquetas de categoría de los paneles ("DEP|", "NOT|", "XXX|"). Solo con
            // barra vertical: con ":" o "-" podría ser parte del título ("Alien: ..."), y
            // entre corchetes también ("[REC]").
        } else {
            // Prefijo desconocido: no se toca. Podría ser parte real del título.
            break;
        }
        text = text[whole_end..].to_string();
    }

    // Corchetes técnicos ("[MicroHD 1080p][ES]"): fuera enteros, antes de que retirar la
    // calidad de dentro deje "[MicroHD ]". Los descriptivos ("[Blanco y Negro]") se quedan.
    text = BRACKETED_RE
        .replace_all(&text, |caps: &regex::Captures| if is_technical_bracket(&caps[1]) { " ".to_string() } else { caps[0].to_string() })
        .into_owned();
    if let Some(m) = SPACED_QUALITY_RE.find(&text) {
        quality.get_or_insert(m.as_str().replace(' ', "").to_uppercase());
        text = remove_span(&text, m.range());
    }

    // Regla 2: marcas de calidad en cualquier posición. La primera es la calidad de la
    // fuente; se retiran todas ("4K HDR" no debe dejar "HDR" en el título).
    while let Some((found, span)) = find_quality(&text) {
        quality.get_or_insert(found);
        text = remove_span(&text, span);
    }

    // Regla 3: año entre paréntesis o corchetes, dentro de rango.
    let mut year = None;
    if let Some(caps) = YEAR_RE.captures(&text) {
        if let Ok(candidate) = caps[1].parse::<i32>() {
            if (1900..=current_year() + 1).contains(&candidate) {
                let whole = caps.get(0).unwrap();
                year = Some(candidate);
                text = remove_span(&text, whole.range());
            }
        }
    }

    // Regla 4: etiquetas de release, se eliminan todas las que aparezcan.
    text = strip_all_release_tags(&text);

    // Año tras guion al final: después de las etiquetas, que en "- 2017_1080p_x264"
    // estaban detrás del año.
    if year.is_none() {
        if let Some(caps) = DASH_YEAR_RE.captures(&text) {
            if let Ok(candidate) = caps[1].parse::<i32>() {
                if (1900..=current_year() + 1).contains(&candidate) {
                    year = Some(candidate);
                    let start = caps.get(0).map(|m| m.start()).unwrap_or(text.len());
                    text.truncate(start);
                }
            }
        }
    }
    // Hasta dos seguidas: "VO-latino".
    for _ in 0..2 {
        let Some(caps) = TRAILING_LANGUAGE_RE.captures(&text) else { break };
        language.get_or_insert(caps[1].to_uppercase());
        let start = caps.get(0).map(|m| m.start()).unwrap_or(text.len());
        text.truncate(start);
    }

    // Regla 5: patrón de episodio (relevante sobre todo para títulos M3U que llegan sin
    // pasar por sources::m3u, p. ej. episodios sueltos). Se retira, no se reextrae aquí:
    // la temporada/número ya se calculó en sources::m3u durante el parseo (F1).
    text = strip_episode_pattern(&text);

    // Regla 6: limpieza final.
    let clean_title = final_cleanup(&text);

    NormalizedTitle { clean_title, year, quality, language }
}

/// Si el contenido de un corchete es solo información técnica (calidad, idioma, release).
fn is_technical_bracket(content: &str) -> bool {
    let upper = content.to_uppercase();
    if upper.contains("V.O.S") {
        return true;
    }
    upper.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).any(|word| {
        QUALITY_MARKERS.contains(&word)
            || LANGUAGE_CODES.contains(&word)
            || TECHNICAL_BRACKET_WORDS.contains(&word)
            || RELEASE_TAGS.iter().any(|tag| *tag == word)
            || (word.ends_with('P') && word.len() >= 4 && word[..word.len() - 1].chars().all(|c| c.is_ascii_digit()))
    })
}

fn find_quality(text: &str) -> Option<(String, std::ops::Range<usize>)> {
    let lower = text.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for marker in QUALITY_MARKERS {
        let needle = marker.to_lowercase();
        if let Some(pos) = find_word(&lower, &needle) {
            if best.is_none_or(|(best_pos, _)| pos < best_pos) {
                best = Some((pos, marker));
            }
        }
    }
    best.map(|(pos, marker)| (marker.to_string(), pos..pos + marker.len()))
}

/// Busca `needle` como palabra completa (no como substring de otra palabra) dentro de
/// `haystack`, ambos ya en minúsculas. `1080p`/`720p` incluyen dígitos y una `p`, así que
/// el límite de palabra se comprueba a mano en vez de con `\b` de regex por marcador.
fn find_word(haystack: &str, needle: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle_len = needle.len();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        let pos = start + rel;
        let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let after_ok = pos + needle_len >= bytes.len() || !bytes[pos + needle_len].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return Some(pos);
        }
        start = pos + 1;
    }
    None
}

fn remove_span(text: &str, span: std::ops::Range<usize>) -> String {
    format!("{}{}", &text[..span.start], &text[span.end..])
}

fn strip_all_release_tags(text: &str) -> String {
    let mut result = text.to_string();
    loop {
        let lower = result.to_lowercase();
        let found = RELEASE_TAGS.iter().find_map(|tag| find_word(&lower, &tag.to_lowercase()).map(|pos| (pos, tag.len())));
        match found {
            Some((pos, len)) => result = remove_span(&result, pos..pos + len),
            None => break,
        }
    }
    result
}

fn strip_episode_pattern(text: &str) -> String {
    for pattern in EPISODE_PATTERNS.iter() {
        if let Some(m) = pattern.find(text) {
            return remove_span(text, m.range());
        }
    }
    text.to_string()
}

fn final_cleanup(text: &str) -> String {
    let text = EMPTY_BRACKETS_RE.replace_all(text, " ");
    let text = FEED_MARKER_RE.replace(&text, "");
    // Un punto pegado a la palabra siguiente es separador de release ("The.Matrix"); con
    // un espacio detrás es puntuación de verdad ("Mr. Link") y se queda.
    let chars: Vec<char> = text.chars().collect();
    let replaced: String = chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            // Entre dígitos tampoco: es una fecha o un número ("13.9.2024").
            let between_digits = i > 0 && chars[i - 1].is_ascii_digit() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            let scene_dot = c == '.' && !between_digits && chars.get(i + 1).is_some_and(|next| !next.is_whitespace());
            if c == '_' || scene_dot { ' ' } else { c }
        })
        .collect();

    let mut cleaned = String::with_capacity(replaced.len());
    let mut last_was_space = false;
    for ch in replaced.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }

    cleaned
        .trim()
        // Sin '(' ')' '[' ']' aquí: un año fuera de rango deja su paréntesis de cierre
        // intacto a propósito (regla 3), y recortarlo a ciegas lo destrozaría.
        .trim_matches(|c: char| c == '-' || c == '|' || c == ':')
        .trim_end_matches(['.', '(', '['])
        .trim()
        .to_string()
}

/// Nombre de categoría sin la decoración que ponen algunos paneles ("== NOVEDADES",
/// "*** CINE ***"), con los espacios repetidos colapsados.
pub fn clean_category_name(raw: &str) -> String {
    let decoration = |c: char| c.is_whitespace() || matches!(c, '=' | '*' | '-' | '#' | '>' | '<' | '|' | '~' | '•');
    let trimmed = raw.trim_matches(decoration);
    if trimmed.is_empty() {
        return raw.trim().to_string();
    }
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn current_year() -> i32 {
    const SECONDS_PER_JULIAN_YEAR: u64 = 31_557_600; // 365.25 días
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    1970 + (secs / SECONDS_PER_JULIAN_YEAR) as i32
}

/// `match_key = slug(clean_title_sin_articulos) + "-" + (year ?? "0000")`.
pub fn match_key(clean_title: &str, year: Option<i32>) -> String {
    let slugged = slug(clean_title);
    let without_article = strip_leading_article_slug(&slugged);
    let year_part = year.map(|y| y.to_string()).unwrap_or_else(|| "0000".to_string());
    format!("{without_article}-{year_part}")
}

fn strip_leading_article_slug(slugged: &str) -> String {
    for article in ARTICLES {
        let prefix = format!("{article}-");
        if let Some(rest) = slugged.strip_prefix(&prefix) {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    slugged.to_string()
}

/// Minúsculas, sin diacríticos, solo `[a-z0-9-]`.
pub fn slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for ch in input.chars() {
        let folded = strip_diacritics(ch.to_ascii_lowercase());
        if folded.is_ascii_alphanumeric() {
            out.push(folded);
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

fn strip_diacritics(ch: char) -> char {
    match ch {
        'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Regla 1: prefijos de idioma/país ──────────────────────────────────────
    #[test]
    fn language_prefix_pipe() {
        let n = normalize("ES | Canal Uno");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.clean_title, "Canal Uno");
    }

    #[test]
    fn language_prefix_brackets() {
        let n = normalize("[ES] Movie Title");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn language_prefix_colon() {
        let n = normalize("ES: Movie");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.clean_title, "Movie");
    }

    #[test]
    fn language_prefix_dash_lat() {
        let n = normalize("LAT - Serie");
        assert_eq!(n.language.as_deref(), Some("LAT"));
        assert_eq!(n.clean_title, "Serie");
    }

    #[test]
    fn panel_category_tags_with_a_pipe_are_removed() {
        assert_eq!(normalize("DEP| Deportes Uno").clean_title, "Deportes Uno");
        assert_eq!(normalize("NOT| 24 Horas").clean_title, "24 Horas");
        assert_eq!(normalize("XXX| Película Adulta (2020)").clean_title, "Película Adulta");
    }

    #[test]
    fn several_prefix_tags_in_a_row_are_removed() {
        let n = normalize("ES| DEP| Fútbol Total HD");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.quality.as_deref(), Some("HD"));
        assert_eq!(n.clean_title, "Fútbol Total");
    }

    #[test]
    fn unknown_prefixes_without_a_pipe_are_kept() {
        assert_eq!(normalize("Alien: Romulus (2024)").clean_title, "Alien: Romulus");
        assert_eq!(normalize("Mission: Impossible").clean_title, "Mission: Impossible");
        assert_eq!(normalize("[REC] (2007)").clean_title, "[REC]");
    }

    #[test]
    fn every_quality_marker_is_removed_and_the_first_one_wins() {
        let n = normalize("Origen (2010) 4K HDR");
        assert_eq!(n.clean_title, "Origen");
        assert_eq!(n.quality.as_deref(), Some("4K"));
        assert_eq!(n.year, Some(2010));
    }

    #[test]
    fn language_prefix_en() {
        let n = normalize("EN | Show");
        assert_eq!(n.language.as_deref(), Some("EN"));
        assert_eq!(n.clean_title, "Show");
    }

    #[test]
    fn hash_prefix_is_stripped_without_language() {
        let n = normalize("#Canal Especial");
        assert_eq!(n.language, None);
        assert_eq!(n.clean_title, "Canal Especial");
    }

    #[test]
    fn vip_prefix_is_discarded_not_assigned() {
        let n = normalize("VIP | Canal Premium");
        assert_eq!(n.language, None);
        assert_eq!(n.quality, None);
        assert_eq!(n.clean_title, "Canal Premium");
    }

    #[test]
    fn unknown_prefix_is_not_stripped() {
        // "XX" no está en ninguna lista conocida: podría ser parte real del título.
        let n = normalize("XX: Something");
        assert_eq!(n.language, None);
        assert_eq!(n.clean_title, "XX: Something");
    }

    // ── Regla 2: calidad ───────────────────────────────────────────────────────
    #[test]
    fn quality_4k_suffix() {
        let n = normalize("Movie Title 4K");
        assert_eq!(n.quality.as_deref(), Some("4K"));
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn quality_1080p() {
        let n = normalize("Movie Title 1080p");
        assert_eq!(n.quality.as_deref(), Some("1080P"));
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn quality_first_match_wins_when_several_present() {
        let n = normalize("Movie FHD HD");
        assert_eq!(n.quality.as_deref(), Some("FHD"));
    }

    #[test]
    fn quality_prefix_after_language_prefix_is_still_found() {
        let n = normalize("ES | 4K - Movie");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.quality.as_deref(), Some("4K"));
        assert_eq!(n.clean_title, "Movie");
    }

    // ── Regla 3: año ───────────────────────────────────────────────────────────
    #[test]
    fn year_in_parens_is_extracted() {
        let n = normalize("Movie Title (2020)");
        assert_eq!(n.year, Some(2020));
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn year_in_brackets_is_extracted() {
        let n = normalize("Movie Title [2020]");
        assert_eq!(n.year, Some(2020));
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn year_out_of_range_low_is_kept_as_title_text() {
        let n = normalize("Movie (1899)");
        assert_eq!(n.year, None);
        assert_eq!(n.clean_title, "Movie (1899)");
    }

    #[test]
    fn year_out_of_range_high_is_kept_as_title_text() {
        let n = normalize("Movie (2200)");
        assert_eq!(n.year, None);
        assert_eq!(n.clean_title, "Movie (2200)");
    }

    #[test]
    fn dangerous_title_1917_is_not_destroyed() {
        let n = normalize("1917");
        assert_eq!(n.year, None);
        assert_eq!(n.clean_title, "1917");
    }

    #[test]
    fn dangerous_title_2012_is_not_destroyed() {
        let n = normalize("2012");
        assert_eq!(n.year, None);
        assert_eq!(n.clean_title, "2012");
    }

    #[test]
    fn dangerous_title_blade_runner_2049_is_not_destroyed() {
        let n = normalize("Blade Runner 2049");
        assert_eq!(n.year, None);
        assert_eq!(n.clean_title, "Blade Runner 2049");
    }

    // ── Regla 4: etiquetas de release ────────────────────────────────────────
    #[test]
    fn release_tag_webdl_is_stripped() {
        let n = normalize("Movie WEB-DL");
        assert_eq!(n.clean_title, "Movie");
    }

    #[test]
    fn release_tags_bluray_and_x264_are_stripped() {
        let n = normalize("Movie BluRay x264");
        assert_eq!(n.clean_title, "Movie");
    }

    #[test]
    fn release_tag_castellano_is_stripped() {
        let n = normalize("Movie CASTELLANO");
        assert_eq!(n.clean_title, "Movie");
    }

    #[test]
    fn multiple_release_tags_are_all_stripped() {
        let n = normalize("Movie DUAL MULTI AC3 DTS");
        assert_eq!(n.clean_title, "Movie");
    }

    // ── Regla 5: patrón de episodio ──────────────────────────────────────────
    #[test]
    fn episode_pattern_sxxexx_is_stripped() {
        let n = normalize("Serie S01E02");
        assert_eq!(n.clean_title, "Serie");
    }

    #[test]
    fn episode_pattern_sx_space_ex_is_stripped() {
        // Visto en un proveedor real (F8): "S4 E5" con espacio, no pegado. Sin este
        // caso, cada episodio quedaba como una "serie" distinta en el catálogo.
        let n = normalize("La Casa de Papel S4 E5");
        assert_eq!(n.clean_title, "La Casa de Papel");
        let n2 = normalize("La Casa de Papel S4 E7");
        assert_eq!(n2.clean_title, n.clean_title, "deben normalizar al mismo título para agruparse");
    }

    #[test]
    fn episode_pattern_nxnn_is_stripped() {
        let n = normalize("Serie 1x02");
        assert_eq!(n.clean_title, "Serie");
    }

    #[test]
    fn episode_pattern_txey_is_stripped() {
        let n = normalize("Serie T1 E2");
        assert_eq!(n.clean_title, "Serie");
    }

    #[test]
    fn episode_pattern_temporada_capitulo_is_stripped() {
        let n = normalize("Serie Temporada 1 Capitulo 2");
        assert_eq!(n.clean_title, "Serie");
    }

    // ── Regla 6: limpieza final ──────────────────────────────────────────────
    #[test]
    fn dots_and_underscores_become_spaces() {
        let n = normalize("Movie.Title_Here");
        assert_eq!(n.clean_title, "Movie Title Here");
    }

    #[test]
    fn double_spaces_are_collapsed() {
        let n = normalize("Movie   Title");
        assert_eq!(n.clean_title, "Movie Title");
    }

    #[test]
    fn trailing_separators_are_trimmed() {
        let n = normalize("Movie Title -");
        assert_eq!(n.clean_title, "Movie Title");
    }

    // ── Casos combinados, títulos reales feos ───────────────────────────────
    #[test]
    fn combined_language_year_release_quality() {
        let n = normalize("ES | Movie Amazing (2021) WEB-DL 1080p");
        assert_eq!(n.language.as_deref(), Some("ES"));
        assert_eq!(n.year, Some(2021));
        assert_eq!(n.quality.as_deref(), Some("1080P"));
        assert_eq!(n.clean_title, "Movie Amazing");
    }

    #[test]
    fn combined_quality_after_year_real_catalog_example() {
        // Ejemplo real visto en un proveedor de verdad (ver docs/DECISIONS.md, F3).
        let n = normalize("Tierra de amor y coraje (2026) 1080p");
        assert_eq!(n.year, Some(2026));
        assert_eq!(n.quality.as_deref(), Some("1080P"));
        assert_eq!(n.clean_title, "Tierra de amor y coraje");
    }

    #[test]
    fn combined_release_tags_with_year_and_language() {
        let n = normalize("[LAT] Otra Pelicula (2019) BluRay DUAL");
        assert_eq!(n.language.as_deref(), Some("LAT"));
        assert_eq!(n.year, Some(2019));
        assert_eq!(n.clean_title, "Otra Pelicula");
    }

    // ── match_key ────────────────────────────────────────────────────────────
    #[test]
    fn match_key_basic_with_year() {
        assert_eq!(match_key("Movie Title", Some(2020)), "movie-title-2020");
    }

    #[test]
    fn match_key_without_year_uses_0000() {
        assert_eq!(match_key("Movie Title", None), "movie-title-0000");
    }

    #[test]
    fn match_key_strips_leading_spanish_article() {
        assert_eq!(match_key("El Padrino", Some(1972)), "padrino-1972");
    }

    #[test]
    fn match_key_strips_leading_english_article() {
        assert_eq!(match_key("The Matrix", Some(1999)), "matrix-1999");
    }

    #[test]
    fn match_key_strips_diacritics() {
        assert_eq!(match_key("Días de Furia", None), "dias-de-furia-0000");
    }

    #[test]
    fn match_key_does_not_over_strip_a_title_that_is_only_an_article() {
        assert_eq!(match_key("La", None), "la-0000");
    }

    #[test]
    fn slug_keeps_only_ascii_alphanumeric_and_dashes() {
        assert_eq!(slug("¡Vámonos, ya! 2"), "vamonos-ya-2");
    }

    // ── Casos de la lista real del usuario (2026-10-19) ───────────────────────
    #[test]
    fn quality_inside_brackets_leaves_no_empty_brackets() {
        assert_eq!(normalize("Sense8(1080p-x265)").clean_title, "Sense8");
        assert_eq!(normalize("La materia oscura(720p-x264)").clean_title, "La materia oscura");
        // Un paréntesis con contenido real se queda.
        assert_eq!(normalize("Snake Eyes (Ojos de serpiente) (1998) 1080p").clean_title, "Snake Eyes (Ojos de serpiente)");
    }

    #[test]
    fn trailing_year_after_a_dash_is_extracted() {
        let n = normalize("Beethoven 2: La familia crece - 1993");
        assert_eq!(n.clean_title, "Beethoven 2: La familia crece");
        assert_eq!(n.year, Some(1993));
        // Sin el guion, un número final sigue siendo parte del título.
        assert_eq!(normalize("Blade Runner 2049").year, None);
        assert_eq!(normalize("1917").clean_title, "1917");
    }

    #[test]
    fn a_dot_followed_by_a_space_is_kept_but_scene_dots_still_become_spaces() {
        assert_eq!(normalize("Mr. Link: El origen perdido (2019)").clean_title, "Mr. Link: El origen perdido");
        assert_eq!(normalize("The.Matrix.(1999)").clean_title, "The Matrix");
    }

    #[test]
    fn dash_year_tolerates_a_trailing_dot_or_release_leftovers() {
        let n = normalize("Big Hero 6 - 2014.");
        assert_eq!((n.clean_title.as_str(), n.year), ("Big Hero 6", Some(2014)));
        let n = normalize("Disobedience - 2017_1080p_x264");
        assert_eq!((n.clean_title.as_str(), n.year), ("Disobedience", Some(2017)));
        // Sin espacio antes del guion es ambiguo ("Formula 1- 2026"): se deja.
        assert_eq!(normalize("Formula 1- 2026").year, None);
    }

    #[test]
    fn trailing_language_words_are_extracted() {
        let n = normalize("Minions: El origen de Gru (2022) 1080p_Latino");
        assert_eq!((n.clean_title.as_str(), n.language.as_deref()), ("Minions: El origen de Gru", Some("LATINO")));
        assert_eq!(normalize("Buscando a Michael (2023) VOSE").clean_title, "Buscando a Michael");
        assert_eq!(normalize("The Judge From Hell 1080P   [V.O.S.E.]").clean_title, "The Judge From Hell");
        // En medio del título, "latino" es parte del título.
        assert_eq!(normalize("Latino Americano (2010)").clean_title, "Latino Americano");
    }

    #[test]
    fn technical_brackets_go_but_descriptive_ones_stay() {
        assert_eq!(normalize("Adrenalina (2022) [WEB-DL m1080p]").clean_title, "Adrenalina");
        assert_eq!(normalize("Bambi 2 (2006) [MicroHD 1080p][ES][Animación]").clean_title, "Bambi 2 [Animación]");
        assert_eq!(normalize("Fate/stay night [Unlimited Blade Works] 1080P").clean_title, "Fate/stay night [Unlimited Blade Works]");
    }

    #[test]
    fn spaced_quality_and_spaced_years_are_recognised() {
        assert_eq!(normalize("Spawn 1080 p").clean_title, "Spawn");
        let n = normalize("Circus of Books ( 2019) 1080p");
        assert_eq!((n.clean_title.as_str(), n.year), ("Circus of Books", Some(2019)));
        assert_eq!(normalize("Àngela y Archie: Tras la Pantalla (2026 )1080p").year, Some(2026));
        assert_eq!(normalize("La maldición de Hill House(The Haunting of Hill House)(1080p-x265").clean_title, "La maldición de Hill House(The Haunting of Hill House)");
    }

    #[test]
    fn bare_resolution_vo_and_dates_from_the_real_catalog() {
        assert_eq!(normalize("Culpable (2021) 1080").clean_title, "Culpable");
        assert_eq!(normalize("Gintama (2006) VO-latino").clean_title, "Gintama");
        assert_eq!(normalize("Cuando el polvo se asienta VO").clean_title, "Cuando el polvo se asienta");
        assert_eq!(normalize("Coca-Cola Music Experience - 13.9.2024 (2024)  720p").clean_title, "Coca-Cola Music Experience - 13.9.2024");
        // Un número que es el título se queda.
        assert_eq!(normalize("2012 (2009)_1080p_x264").clean_title, "2012");
    }

    #[test]
    fn feed_markers_at_the_end_of_channel_names_are_removed() {
        assert_eq!(normalize("DAZN LaLiga HD@").clean_title, "DAZN LaLiga");
        assert_eq!(normalize("M+ LaLiga TV HD ?").clean_title, "M+ LaLiga TV");
        // Signos que son parte del nombre se quedan.
        assert_eq!(normalize("¿Qué te juegas? - 2019").clean_title, "¿Qué te juegas?");
        assert_eq!(normalize("Movistar Plus+ HD").clean_title, "Movistar Plus+");
    }

    #[test]
    fn category_decoration_from_the_panel_is_removed() {
        assert_eq!(clean_category_name("== NOVEDADES"), "NOVEDADES");
        assert_eq!(clean_category_name("== FORMULA 1 y  MOTO GP"), "FORMULA 1 y MOTO GP");
        assert_eq!(clean_category_name("*** CINE ***"), "CINE");
        assert_eq!(clean_category_name("Peliculas 2024"), "Peliculas 2024");
        assert_eq!(clean_category_name("==="), "===", "si no queda nada, mejor el original que vacío");
    }
}

