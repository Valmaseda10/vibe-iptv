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

/// Serializa los refrescos de sesión dentro de este proceso. Supabase invalida el
/// refresh token en cuanto se usa una vez (rotación); si dos tareas en segundo plano
/// (varios `push_*` de sincronización disparados casi a la vez) intentaran refrescar a
/// la vez con el mismo token, la que llegase segunda recibiría un rechazo legítimo de
/// Supabase por un token ya consumido — no porque la sesión esté realmente muerta — y
/// sin este cerrojo esa tarea perdedora borraría una sesión que la otra acababa de
/// renovar con éxito. Con el cerrojo, la segunda espera, y al entrar ve la sesión ya
/// refrescada por la primera y no necesita tocar la red.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let _guard = REFRESH_LOCK.lock().await;

    // Se relee ya con el cerrojo cogido: si otra tarea estaba refrescando cuando
    // llegamos, para cuando nos toca turno el keychain puede tener una sesión más nueva
    // que la que había cuando decidimos llamar a esta función.
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
            // Antes de dar la sesión por muerta: puede que este rechazo se deba a que
            // el refresh token ya se consumió en otro refresco legítimo (rotación de
            // Supabase) ocurrido justo antes de que cogiéramos el cerrojo. Si el
            // keychain tiene ahora un token de refresco distinto y aún vigente, esa
            // sesión es la buena y no hay que borrar nada.
            match load() {
                Some(current)
                    if current.refresh_token != session.refresh_token
                        && current.expires_at > crate::db::now_epoch() =>
                {
                    Some(current)
                }
                _ => {
                    // El refresh token ya no vale de verdad (revocado, o la sesión
                    // llevaba demasiado sin usarse): hay que volver a iniciar sesión.
                    clear();
                    None
                }
            }
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

    /// Camino sin red: si la sesión guardada aún no está cerca de caducar, `get_valid`
    /// la devuelve tal cual sin llamar a Supabase.
    #[tokio::test]
    async fn get_valid_returns_fresh_session_without_refreshing() {
        let _cleanup = Cleanup;
        let session = Session {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            user_id: "u1".into(),
            email: "demo@example.com".into(),
            expires_at: crate::db::now_epoch() + 3600,
        };
        save(&session);
        let valid = get_valid().await.unwrap();
        assert_eq!(valid.access_token, "at");
    }

    #[tokio::test]
    async fn get_valid_returns_none_when_nothing_saved() {
        clear();
        assert!(get_valid().await.is_none());
    }
}
