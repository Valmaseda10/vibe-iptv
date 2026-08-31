//! Control parental (F8): un PIN por perfil que, mientras no se introduzca, oculta las
//! categorías marcadas como adultas (`categories.is_adult`) de las listas de canales y
//! películas — series no tiene columna de categoría (ver `docs/DATA-MODEL.md`), así que
//! no hay nada que ocultar ahí a este nivel.
//!
//! El PIN en sí vive hasheado en `profiles.pin_hash` (persistente); si el desbloqueo
//! está vigente para esta sesión de la app es un estado en memoria (`ParentalGuard`),
//! nunca se guarda "desbloqueado" en disco — cerrar la app vuelve a pedir el PIN.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

/// Compartido entre los comandos Tauri (que desbloquean con el PIN) y el servidor HTTP
/// (que lo consulta en cada lectura de categorías/canales/películas).
#[derive(Default)]
pub struct ParentalGuard {
    unlocked: AtomicBool,
}

impl ParentalGuard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }

    pub fn unlock(&self) {
        self.unlocked.store(true, Ordering::SeqCst);
    }

    pub fn lock(&self) {
        self.unlocked.store(false, Ordering::SeqCst);
    }
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

    #[test]
    fn guard_starts_locked_and_toggles() {
        let guard = ParentalGuard::new();
        assert!(!guard.is_unlocked());
        guard.unlock();
        assert!(guard.is_unlocked());
        guard.lock();
        assert!(!guard.is_unlocked());
    }
}
