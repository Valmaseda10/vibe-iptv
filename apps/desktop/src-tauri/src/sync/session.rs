//! Guarda la sesión de Supabase (tokens) en el keychain del sistema — no es una
//! credencial de proveedor IPTV, pero sigue siendo un secreto personal que no debe
//! vivir en SQLite en claro (mismo criterio de higiene que la regla R5).

use super::supabase::{Session, SupabaseClient, SupabaseError};
use crate::util::keychain;

const CREDENTIAL_REF: &str = "supabase-session";
/// Refresca un poco antes de que caduque de verdad, para no arriesgarse a que una
/// petición real falle por segundos de diferencia entre el reloj local y el de
/// Supabase.
const REFRESH_MARGIN_SECONDS: i64 = 60;

pub fn save(session: &Session) {
    if let Ok(json) = serde_json::to_string(session) {
        if let Err(err) = keychain::store_secret(CREDENTIAL_REF, &json) {
            tracing::warn!(error = %err, "no se pudo guardar la sesión de sincronización");
        }
    }
}

pub fn load() -> Option<Session> {
    let json = keychain::read_secret(CREDENTIAL_REF).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn clear() {
    let _ = keychain::delete_secret(CREDENTIAL_REF);
}

/// Sesión guardada, refrescada primero si está a punto de caducar. `None` si no hay
/// ninguna sesión (no ha iniciado sesión, o cerró sesión) — nunca es un error, es un
/// estado válido: la sincronización es opcional, la app funciona sin cuenta.
pub async fn get_valid() -> Option<Session> {
    let session = load()?;
    if session.expires_at > crate::db::now_epoch() + REFRESH_MARGIN_SECONDS {
        return Some(session);
    }

    match SupabaseClient::new().refresh(&session.refresh_token).await {
        Ok(refreshed) => {
            save(&refreshed);
            Some(refreshed)
        }
        Err(SupabaseError::Api(_)) => {
            // El refresh token ya no vale (revocado, o la sesión llevaba demasiado sin
            // usarse): hay que volver a iniciar sesión, no tiene sentido conservar algo
            // que ya no funciona.
            clear();
            None
        }
        Err(_) => Some(session), // fallo de red puntual: se sigue intentando con la sesión vieja mientras no caduque del todo.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Borra el secreto de prueba del keychain real al terminar, pase lo que pase
    /// (mismo criterio que otros tests de `util::keychain`: no hay backend de prueba).
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            clear();
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let _cleanup = Cleanup;
        let session = Session {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            user_id: "u1".into(),
            email: "demo@example.com".into(),
            expires_at: crate::db::now_epoch() + 3600,
        };
        save(&session);
        let loaded = load().unwrap();
        assert_eq!(loaded.access_token, "at");
        assert_eq!(loaded.email, "demo@example.com");
    }

    #[test]
    fn load_returns_none_when_nothing_saved() {
        clear(); // por si un test anterior dejó algo.
        assert!(load().is_none());
    }
}
