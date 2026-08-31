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

#[cfg(test)]
mod tests {
    use super::*;

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
