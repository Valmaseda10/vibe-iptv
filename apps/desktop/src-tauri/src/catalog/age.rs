//! Clasificación por edad (F14): de la certificación que da TMDB ("12", "PG-13", "TP",
//! "TV-MA"…) a la edad mínima, para poder filtrar en SQL qué ve un perfil infantil.
//!
//! TMDB da la de España si existe y, si no, la de EE. UU. o la primera que tenga (ver
//! `tmdb::pick_certification`), sin decir de qué país es: por eso se reconocen los
//! sistemas más habituales a la vez. Lo que no se reconoce es `None`, y un título sin edad
//! conocida nunca se trata como apto (ver migración 017).

/// Edad mínima de una certificación, o `None` si no se reconoce.
pub fn min_age(rating: &str) -> Option<i64> {
    let normalized: String = rating.trim().to_uppercase().chars().filter(|c| !c.is_whitespace()).collect();
    let known = match normalized.as_str() {
        // España: "A"/"TP"/"APTA" (todos los públicos), "AI" (apta, especialmente
        // recomendada para la infancia), "X" (pornografía).
        "A" | "TP" | "APTA" | "AI" | "ATP" => Some(0),
        "X" => Some(18),
        // EE. UU. (cine): PG es "con supervisión", se trata como 7.
        "G" => Some(0),
        "PG" => Some(7),
        "PG-13" => Some(13),
        "R" => Some(17),
        "NC-17" => Some(18),
        // EE. UU. (televisión).
        "TV-Y" | "TV-G" => Some(0),
        "TV-Y7" | "TV-Y7-FV" => Some(7),
        "TV-PG" => Some(10),
        "TV-14" => Some(14),
        "TV-MA" => Some(17),
        // Reino Unido.
        "U" | "UC" => Some(0),
        "12A" => Some(12),
        "R18" => Some(18),
        // Francia/otros: "TOUS PUBLICS", "-10", "-12"...
        "TOUSPUBLICS" | "ALL" | "L" | "LIVRE" => Some(0),
        _ => None,
    };
    if known.is_some() {
        return known;
    }
    // Numéricas en cualquier sistema: "7", "12", "16", "18", "-16", "16+", "FSK 12".
    let digits: String = normalized.chars().filter(|c| c.is_ascii_digit()).collect();
    let starts_numeric = normalized.trim_start_matches(['-', '+']).trim_start_matches("FSK").starts_with(|c: char| c.is_ascii_digit());
    match digits.parse::<i64>() {
        Ok(age) if starts_numeric && age <= 21 => Some(age),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_ratings() {
        assert_eq!(min_age("TP"), Some(0));
        assert_eq!(min_age("A"), Some(0));
        assert_eq!(min_age("Ai"), Some(0));
        assert_eq!(min_age("7"), Some(7));
        assert_eq!(min_age("12"), Some(12));
        assert_eq!(min_age("16"), Some(16));
        assert_eq!(min_age("18"), Some(18));
        assert_eq!(min_age("X"), Some(18));
    }

    #[test]
    fn us_film_and_tv_ratings() {
        assert_eq!(min_age("G"), Some(0));
        assert_eq!(min_age("PG"), Some(7));
        assert_eq!(min_age("PG-13"), Some(13));
        assert_eq!(min_age("R"), Some(17));
        assert_eq!(min_age("NC-17"), Some(18));
        assert_eq!(min_age("TV-Y7"), Some(7));
        assert_eq!(min_age("TV-14"), Some(14));
        assert_eq!(min_age("TV-MA"), Some(17));
    }

    #[test]
    fn other_numeric_systems() {
        assert_eq!(min_age("12A"), Some(12));
        assert_eq!(min_age("-16"), Some(16));
        assert_eq!(min_age("16+"), Some(16));
        assert_eq!(min_age("FSK 12"), Some(12));
        assert_eq!(min_age(" 16 "), Some(16));
    }

    #[test]
    fn unknown_ratings_are_not_guessed() {
        assert_eq!(min_age(""), None);
        assert_eq!(min_age("NR"), None);
        assert_eq!(min_age("Unrated"), None);
        assert_eq!(min_age("2016"), None, "un año no es una edad");
    }
}
