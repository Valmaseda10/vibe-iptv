//! Conversión entre epoch (segundos UTC) y fecha civil, sin depender de una librería de
//! fechas para un problema tan acotado — mismo principio que `redact()`/
//! `catalog::normalize`: código propio pequeño y bien testeado en vez de una
//! dependencia para algo de forma fija. Usado por `catalog::epg` (parsear XMLTV) y por
//! `sources::xtream` (construir URLs de catch-up, F9), que necesitan las dos
//! direcciones de la misma conversión.
//!
//! Algoritmo de Howard Hinnant, dominio público:
//! https://howardhinnant.github.io/date_algorithms.html

/// Días desde 1970-01-01 hasta la fecha civil dada.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64 + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Inverso de [`days_from_civil`]: día civil (año, mes, día) para los días transcurridos
/// desde 1970-01-01.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y } as i32;
    (y, m, d)
}

/// Año, mes, día, hora, minuto y segundo (UTC) de un instante epoch.
pub fn epoch_to_ymd_hms(epoch: i64) -> (i32, u32, u32, i64, i64, i64) {
    let days = epoch.div_euclid(86_400);
    let secs_of_day = epoch.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60)
}

/// Instante epoch en RFC 3339 UTC (`2023-11-14T22:13:20Z`), el formato de marca de
/// tiempo que esperan las APIs de la nube (`sync`).
pub fn epoch_to_rfc3339(epoch: i64) -> String {
    let (y, m, d, h, mi, s) = epoch_to_ymd_hms(epoch);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// RFC 3339 a epoch, truncando a segundos. Ignora fracciones y sufijo de zona: los seis
/// primeros grupos de dígitos son siempre año, mes, día, hora, minuto y segundo, y todo
/// lo que llega aquí viene en UTC (`Z` o `+00:00`).
pub fn rfc3339_to_epoch(value: &str) -> Option<i64> {
    let digits: Vec<i64> = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(6)
        .filter_map(|s| s.parse().ok())
        .collect();
    let [y, mo, d, h, mi, s] = digits[..] else { return None };
    let days = days_from_civil(y as i32, mo as u32, d as u32)?;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_roundtrips_and_ignores_fraction_and_offset() {
        assert_eq!(epoch_to_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339_to_epoch("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        // Forma real de Firestore (milisegundos) y de Postgres (microsegundos + desfase).
        assert_eq!(rfc3339_to_epoch("2023-11-14T22:13:20.180Z"), Some(1_700_000_000));
        assert_eq!(rfc3339_to_epoch("2023-11-14T22:13:20.123456+00:00"), Some(1_700_000_000));
        assert_eq!(rfc3339_to_epoch("no es una fecha"), None);
        assert_eq!(rfc3339_to_epoch(""), None);
    }

    #[test]
    fn days_from_civil_matches_known_epoch_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 1, 1), Some(10957));
        assert_eq!(days_from_civil(2020, 1, 1), Some(18262));
    }

    #[test]
    fn civil_from_days_is_the_inverse_of_days_from_civil() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2020, 12, 31), (2026, 8, 31)] {
            let days = days_from_civil(y, m, d).unwrap();
            assert_eq!(civil_from_days(days), (y, m, d));
        }
    }

    #[test]
    fn epoch_to_ymd_hms_roundtrips_a_known_instant() {
        // 2020-01-01T12:34:56Z
        let epoch = 18262 * 86_400 + 12 * 3600 + 34 * 60 + 56;
        assert_eq!(epoch_to_ymd_hms(epoch), (2020, 1, 1, 12, 34, 56));
    }
}
