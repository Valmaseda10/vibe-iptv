// Sin consumidores hasta la fase F1, cuando sources::xtream y sources::m3u empiecen a
// registrar URLs de proveedor.
#![allow(dead_code)]

use reqwest::Url;

const MASK: &str = "REDACTED";

/// Enmascara usuario, contraseña y query string de una URL de proveedor antes de que
/// pueda llegar a un log, un mensaje de error o un nombre de fichero de caché.
///
/// Cubre tres formas en las que un panel Xtream esconde credenciales: en el userinfo
/// (`user:pass@host`), en segmentos de ruta (`/live/<user>/<pass>/id.ts`) y en la query
/// (`get.php?username=...&password=...`). Si la cadena no es una URL válida, aplica un
/// respaldo basado en los mismos patrones sobre el texto crudo.
pub fn redact(input: &str) -> String {
    match Url::parse(input) {
        Ok(mut url) if !url.cannot_be_a_base() => {
            redact_userinfo(&mut url);
            redact_credential_path_segments(&mut url);
            redact_query(&mut url);
            url.to_string()
        }
        _ => redact_fallback(input),
    }
}

fn redact_userinfo(url: &mut Url) {
    if !url.username().is_empty() {
        let _ = url.set_username(MASK);
    }
    if url.password().is_some() {
        let _ = url.set_password(Some(MASK));
    }
}

/// Los paneles Xtream codifican usuario y contraseña como segmentos de ruta:
/// `/live/<usuario>/<contraseña>/<id>.<ext>`, igual para `/movie/` y `/series/`.
fn redact_credential_path_segments(url: &mut Url) {
    let Some(segments) = url.path_segments() else {
        return;
    };
    let segments: Vec<String> = segments.map(str::to_owned).collect();

    let is_credential_path = matches!(segments.first().map(String::as_str), Some("live" | "movie" | "series"))
        && segments.len() >= 3;
    if !is_credential_path {
        return;
    }

    let mut redacted = segments;
    redacted[1] = MASK.to_string();
    redacted[2] = MASK.to_string();

    if let Ok(mut path_mut) = url.path_segments_mut() {
        path_mut.clear();
        for segment in redacted {
            path_mut.push(&segment);
        }
    }
}

fn redact_query(url: &mut Url) {
    if url.query().is_some() {
        url.set_query(Some(MASK));
    }
}

/// Respaldo cuando la cadena no es una URL válida en sentido RFC 3986 (falta el
/// esquema, por ejemplo). Aplica los mismos patrones sobre el texto por posición de
/// separadores, sin asumir estructura de URL.
fn redact_fallback(input: &str) -> String {
    let (before_query, query) = match input.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (input, None),
    };

    let masked_base = redact_path_segments_fallback(&redact_userinfo_fallback(before_query));

    match query {
        Some(_) => format!("{masked_base}?{MASK}"),
        None => masked_base,
    }
}

fn redact_userinfo_fallback(s: &str) -> String {
    let Some(at_pos) = s.find('@') else {
        return s.to_string();
    };

    if let Some(scheme_end) = s.find("://") {
        let authority_start = scheme_end + 3;
        if at_pos > authority_start {
            return format!("{}{}@{}", &s[..authority_start], MASK, &s[at_pos + 1..]);
        }
    } else if s.find('/').is_none_or(|slash_pos| at_pos < slash_pos) {
        return format!("{MASK}@{}", &s[at_pos + 1..]);
    }

    s.to_string()
}

fn redact_path_segments_fallback(s: &str) -> String {
    let segments: Vec<&str> = s.split('/').collect();
    for (i, segment) in segments.iter().enumerate() {
        if matches!(*segment, "live" | "movie" | "series") && segments.len() > i + 2 {
            let mut owned: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
            owned[i + 1] = MASK.to_string();
            owned[i + 2] = MASK.to_string();
            return owned.join("/");
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redacts_userinfo_credentials() {
        let out = redact("http://juan:hunter2@panel.example.com/get.php");
        assert!(!out.contains("juan"));
        assert!(!out.contains("hunter2"));
        assert!(out.contains("REDACTED:REDACTED@panel.example.com"));
    }

    #[test]
    fn redacts_xtream_get_php_query_credentials() {
        let out = redact(
            "http://panel.example.com:8080/get.php?username=juan&password=hunter2&type=m3u_plus&output=ts",
        );
        assert!(!out.contains("juan"));
        assert!(!out.contains("hunter2"));
        assert!(out.contains("get.php?REDACTED"));
    }

    #[test]
    fn redacts_xtream_live_path_segments() {
        let out = redact("http://panel.example.com:8080/live/juan/hunter2/12345.ts");
        assert!(!out.contains("juan"));
        assert!(!out.contains("hunter2"));
        assert!(out.contains("/live/REDACTED/REDACTED/12345.ts"));
    }

    #[test]
    fn redacts_xtream_movie_and_series_path_segments() {
        let movie = redact("http://panel.example.com/movie/ana/clave123/9.mkv");
        assert!(!movie.contains("ana"));
        assert!(!movie.contains("clave123"));

        let series = redact("http://panel.example.com/series/ana/clave123/9.mp4");
        assert!(!series.contains("ana"));
        assert!(!series.contains("clave123"));
    }

    #[test]
    fn leaves_urls_without_credentials_mostly_intact() {
        let out = redact("http://panel.example.com/img/logo.png");
        assert_eq!(out, "http://panel.example.com/img/logo.png");
    }

    #[test]
    fn falls_back_when_scheme_is_missing() {
        let out = redact("panel.example.com:8080/live/juan/hunter2/1.ts?token=abc");
        assert!(!out.contains("juan"));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("token=abc"));
        assert!(out.contains("/live/REDACTED/REDACTED/1.ts"));
    }

    #[test]
    fn never_panics_on_degenerate_input() {
        for input in ["", "?", "@", "///", "http://"] {
            let _ = redact(input);
        }
    }
}
