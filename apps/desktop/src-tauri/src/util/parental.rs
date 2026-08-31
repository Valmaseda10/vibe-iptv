//! Control parental por perfil (sustituye al PIN único y global de F8 — ver
//! docs/DECISIONS.md, "perfiles Netflix"): cada perfil puede tener su propio PIN de
//! entrada (`profiles.pin_hash`), y un perfil marcado como infantil (`adult_ok = 0`)
//! nunca ve categorías adultas (`categories.is_adult`), sin PIN de por medio. Ya no
//! hace falta un `ParentalGuard` en memoria — el PIN se comprueba una sola vez al
//! entrar en el perfil (`server::profiles::post_enter_profile`), no en cada petición.

use sha2::{Digest, Sha256};

/// Hash de un PIN para guardar en `profiles.pin_hash`. No es para contraseñas de
/// verdad (un PIN corto no lo necesita, y esto no pretende serlo): solo evita guardarlo
/// en claro en SQLite.
pub fn hash_pin(pin: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pin.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn verify_pin(pin: &str, stored_hash: &str) -> bool {
    hash_pin(pin) == stored_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_pin_is_deterministic_and_never_stores_the_pin_in_clear() {
        let hash = hash_pin("1234");
        assert_eq!(hash, hash_pin("1234"));
        assert!(!hash.contains("1234"));
        assert_eq!(hash.len(), 64); // sha256 en hexadecimal
    }

    #[test]
    fn verify_pin_only_accepts_the_right_pin() {
        let hash = hash_pin("1234");
        assert!(verify_pin("1234", &hash));
        assert!(!verify_pin("0000", &hash));
    }
}
