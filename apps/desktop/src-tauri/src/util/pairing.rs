//! Emparejamiento de dispositivos para el modo LAN (F10). Dos secretos distintos, con
//! vidas distintas:
//!
//! - **Código de 6 dígitos**: de un solo uso, vive unos minutos, solo en memoria
//!   ([`PairingState`]). Es lo que el usuario lee en pantalla y teclea en el otro
//!   dispositivo — corto a propósito, para que sea cómodo de escribir con un mando.
//! - **Token del dispositivo**: se entrega una única vez al canjear el código, y a
//!   partir de ahí identifica a ese dispositivo para siempre (hasta que se revoque).
//!   Solo se guarda su hash (regla R5, mismo criterio que `util::parental` con el PIN).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::RngExt;
use sha2::{Digest, Sha256};

/// Cuánto vive un código antes de caducar. Corto: es un secreto de un solo uso pensado
/// para teclearse en el momento, no para guardarse ni compartirse.
const PAIRING_CODE_TTL: Duration = Duration::from_secs(5 * 60);

/// Hash de un token de dispositivo para guardar en `paired_devices.token_hash`.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Token opaco de dispositivo: 32 bytes de `rand`'s CSPRNG por defecto, en hexadecimal.
/// Nunca se recupera después de entregarlo (igual que una clave de API): si se pierde,
/// hay que revocar el dispositivo y volver a emparejarlo.
pub fn generate_device_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_six_digit_code() -> String {
    format!("{:06}", rand::rng().random_range(0..1_000_000))
}

struct PendingCode {
    code: String,
    expires_at: Instant,
}

/// Un único código pendiente a la vez a propósito: generar uno nuevo invalida el
/// anterior, así que no hay que preocuparse por códigos viejos olvidados en pantalla.
/// Vive en memoria, como `ParentalGuard` — un reinicio de la app cancela cualquier
/// emparejamiento a medias, que es justo lo que se quiere (nadie debería estar
/// emparejando un dispositivo nuevo sin la app delante).
#[derive(Default)]
pub struct PairingState {
    pending: Mutex<Option<PendingCode>>,
}

impl PairingState {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    /// Genera un código nuevo y descarta cualquier pendiente anterior. Devuelve el
    /// código (para enseñarlo en pantalla) y cuándo caduca.
    pub fn start(&self) -> (String, Instant) {
        let code = generate_six_digit_code();
        let expires_at = Instant::now() + PAIRING_CODE_TTL;
        *self.pending.lock().unwrap_or_else(|p| p.into_inner()) = Some(PendingCode { code: code.clone(), expires_at });
        (code, expires_at)
    }

    /// Comprueba el código pendiente. Solo se consume (deja de valer) cuando acierta o
    /// cuando ha caducado — una tecleada equivocada no quema el código: quien esté
    /// emparejando un mando a distancia puede volver a intentarlo sin tener que volver
    /// a la pantalla del PC a pedir uno nuevo, mientras siga dentro del plazo.
    pub fn redeem(&self, code: &str) -> bool {
        let mut guard = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(pending) if pending.expires_at < Instant::now() => {
                *guard = None; // caducado: se limpia de paso, no vale ni aunque acierte.
                false
            }
            Some(pending) if pending.code == code => {
                *guard = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_token_is_deterministic_and_never_stores_the_token_in_clear() {
        let token = generate_device_token();
        let hash = hash_token(&token);
        assert_eq!(hash, hash_token(&token));
        assert!(!hash.contains(&token));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn generate_device_token_does_not_repeat_across_calls() {
        assert_ne!(generate_device_token(), generate_device_token());
    }

    #[test]
    fn six_digit_code_is_always_six_digits() {
        for _ in 0..50 {
            let code = generate_six_digit_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn redeem_accepts_the_right_code_once_and_rejects_it_afterwards() {
        let state = PairingState::default();
        let (code, _expires_at) = state.start();

        assert!(state.redeem(&code), "el código recién generado debería ser válido");
        assert!(!state.redeem(&code), "el mismo código no debería valer una segunda vez");
    }

    #[test]
    fn redeem_rejects_a_wrong_code() {
        let state = PairingState::default();
        let (code, _) = state.start();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert!(!state.redeem(wrong));
    }

    #[test]
    fn a_wrong_guess_does_not_burn_the_code_a_later_correct_guess_still_works() {
        let state = PairingState::default();
        let (code, _) = state.start();
        let wrong = if code == "000000" { "111111" } else { "000000" };

        assert!(!state.redeem(wrong), "una tecleada equivocada no debe acertar");
        assert!(state.redeem(&code), "pero el código sigue vivo para reintentar");
    }

    #[test]
    fn starting_a_new_code_invalidates_the_previous_one() {
        let state = PairingState::default();
        let (first, _) = state.start();
        let (second, _) = state.start();
        assert_ne!(first, second, "es un caso extremo con 1 entre un millón de probabilidad si coinciden");
        assert!(!state.redeem(&first), "el código viejo ya no debería valer tras pedir uno nuevo");
        assert!(state.redeem(&second));
    }

    #[test]
    fn redeem_rejects_an_expired_code() {
        // No se puede avanzar `Instant` de verdad sin dormir; en vez de eso se
        // construye un `PendingCode` ya caducado directamente para probar esa rama sin
        // depender de un `sleep` real que ralentizaría la prueba.
        let state = PairingState::default();
        let expired = PendingCode { code: "123456".to_string(), expires_at: Instant::now() - Duration::from_secs(1) };
        *state.pending.lock().unwrap() = Some(expired);

        assert!(!state.redeem("123456"));
    }
}
