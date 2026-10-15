//! Guarda la sesión de la cuenta en la nube en el keychain del sistema: no es una
//! credencial de proveedor IPTV, pero sigue siendo un secreto personal que no debe vivir
//! en SQLite en claro (mismo criterio de higiene que la regla R5).
//!
//! Local-first (F11): la sesión solo se borra cuando la nube dice expresamente que ya
//! no vale. Sin red, o con el servidor caído o pausado, se sigue usando la que hay —
//! antes un fallo así cerraba la sesión y dejaba al usuario fuera de su propia app.

use super::backend::{CloudBackend, CloudError, Session};
use crate::util::keychain;

/// Los tests usan su propia entrada: antes compartían la de la app y, al ejecutarlos,
/// cerraban la sesión real de quien estuviera desarrollando.
#[cfg(not(test))]
const CREDENTIAL_REF: &str = "cloud-session";
#[cfg(test)]
const CREDENTIAL_REF: &str = "cloud-session-test";

/// Sesión de Supabase de antes de F11. Ya no sirve para nada; se borra al arrancar.
const LEGACY_CREDENTIAL_REF: &str = "supabase-session";

/// Refresca un poco antes de que caduque de verdad, para no arriesgarse a que una
/// petición falle por segundos de diferencia entre el reloj local y el del servidor.
const REFRESH_MARGIN_SECONDS: i64 = 60;

/// Serializa los refrescos dentro de este proceso. Si dos tareas refrescaran a la vez
/// con el mismo token de refresco, y el servidor lo rotara en el primer uso, la segunda
/// recibiría un rechazo legítimo y borraría una sesión que la primera acababa de
/// renovar. Con el cerrojo, la segunda espera y relee la sesión ya renovada.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Lo que va al keychain: solo lo que dura. El Administrador de credenciales de Windows
/// admite 2560 bytes por secreto y `keyring` los guarda en UTF-16; una sesión de
/// Firebase completa ocupa unos 2600 (el token de acceso, un JWT, ronda los 950
/// caracteres) y se rechazaba sin que nada lo notara. Encontrado probando contra el
/// proyecto real: con los tokens cortos del backend falso de los tests nunca pasaba.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredSession {
    refresh_token: String,
    user_id: String,
    email: String,
}

/// El token de acceso dura una hora: vive solo en memoria. Tras reiniciar la app no
/// hay ninguno y `get_valid` lo renueva con el de refresco.
static ACCESS_TOKEN: std::sync::Mutex<Option<(String, i64)>> = std::sync::Mutex::new(None);

fn access_token_slot() -> std::sync::MutexGuard<'static, Option<(String, i64)>> {
    ACCESS_TOKEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn save(session: &Session) {
    let stored = StoredSession { refresh_token: session.refresh_token.clone(), user_id: session.user_id.clone(), email: session.email.clone() };
    if let Ok(json) = serde_json::to_string(&stored) {
        if let Err(err) = keychain::store_secret(CREDENTIAL_REF, &json) {
            tracing::warn!(error = %err, "no se pudo guardar la sesión de la cuenta");
        }
    }
    *access_token_slot() = Some((session.access_token.clone(), session.expires_at));
}

/// La sesión guardada. Sin token de acceso en memoria, sale con `expires_at = 0`: la
/// primera llamada a `get_valid` lo renueva.
pub fn load() -> Option<Session> {
    let json = keychain::read_secret(CREDENTIAL_REF).ok()?;
    let stored: StoredSession = serde_json::from_str(&json).ok()?;
    let (access_token, expires_at) = access_token_slot().clone().unwrap_or_default();
    Some(Session { access_token, refresh_token: stored.refresh_token, user_id: stored.user_id, email: stored.email, expires_at })
}

pub fn clear() {
    let _ = keychain::delete_secret(CREDENTIAL_REF);
    *access_token_slot() = None;
}

/// Borra la sesión de Supabase que pudiera quedar de antes de F11.
pub fn clear_legacy() {
    let _ = keychain::delete_secret(LEGACY_CREDENTIAL_REF);
}

/// Sesión guardada, refrescada primero si está a punto de caducar. `None` solo si no
/// hay sesión o si la nube confirma que ya no vale. Si el refresco falla por algo
/// pasajero (sin red, servidor caído), devuelve la sesión que había: sus peticiones
/// fallarán y se reintentarán, pero el usuario sigue dentro.
pub async fn get_valid(cloud: &dyn CloudBackend) -> Option<Session> {
    let _guard = REFRESH_LOCK.lock().await;

    // Se relee ya con el cerrojo cogido: otra tarea pudo renovarla mientras esperábamos.
    let session = load()?;
    if session.expires_at > crate::db::now_epoch() + REFRESH_MARGIN_SECONDS {
        return Some(session);
    }

    match cloud.refresh(&session).await {
        Ok(refreshed) => {
            save(&refreshed);
            Some(refreshed)
        }
        Err(CloudError::SessionExpired) => {
            clear();
            None
        }
        Err(err) => {
            tracing::info!(error = %err, "no se pudo renovar la sesión; se sigue con la actual");
            Some(session)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::memory::MemoryBackend;
    use crate::util::keychain::TEST_LOCK as TEST_KEYCHAIN_LOCK;

    /// Borra el secreto de prueba del keychain real al terminar, pase lo que pase.
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            clear();
        }
    }

    fn session(expires_in: i64) -> Session {
        Session {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            user_id: "u1".into(),
            email: "demo@example.com".into(),
            expires_at: crate::db::now_epoch() + expires_in,
        }
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        save(&session(3600));
        let loaded = load().unwrap();
        assert_eq!(loaded.access_token, "at");
        assert_eq!(loaded.email, "demo@example.com");
    }

    /// Tamaños reales de una sesión de Firebase (token de acceso de ~950 caracteres): la
    /// sesión completa no cabía en el keychain de Windows y nunca llegaba a guardarse.
    #[tokio::test]
    async fn a_full_size_firebase_session_is_saved() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        let real_size = Session { access_token: "a".repeat(1_200), refresh_token: "r".repeat(300), ..session(3600) };
        save(&real_size);
        let loaded = load().expect("la sesión debe poder guardarse");
        assert_eq!(loaded.refresh_token.len(), 300);
        assert_eq!(loaded.access_token.len(), 1_200);
    }

    #[tokio::test]
    async fn get_valid_returns_none_when_nothing_saved() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        clear();
        assert!(get_valid(&MemoryBackend::new()).await.is_none());
    }

    #[tokio::test]
    async fn get_valid_returns_a_fresh_session_without_refreshing() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        save(&session(3600));
        let cloud = MemoryBackend::new();
        cloud.fail_everything(Some(CloudError::Network));
        assert_eq!(get_valid(&cloud).await.unwrap().access_token, "at");
    }

    #[tokio::test]
    async fn an_expiring_session_is_refreshed_and_saved() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        save(&session(10));
        let refreshed = get_valid(&MemoryBackend::new()).await.unwrap();
        assert_ne!(refreshed.access_token, "at");
        assert_eq!(load().unwrap().access_token, refreshed.access_token);
    }

    /// El fallo real que motivó F11: el proyecto de Supabase se pausaba, el refresco
    /// fallaba y la sesión se borraba, dejando al usuario fuera de la app.
    #[tokio::test]
    async fn an_outage_while_refreshing_keeps_the_user_signed_in() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        save(&session(10));
        for outage in [CloudError::Network, CloudError::Unavailable, CloudError::InvalidResponse] {
            let cloud = MemoryBackend::new();
            cloud.fail_everything(Some(outage));
            assert!(get_valid(&cloud).await.is_some());
            assert!(load().is_some());
        }
    }

    #[tokio::test]
    async fn a_revoked_session_is_cleared() {
        let _lock = TEST_KEYCHAIN_LOCK.lock().await;
        let _cleanup = Cleanup;
        save(&session(10));
        let cloud = MemoryBackend::new();
        cloud.fail_refresh(Some(CloudError::SessionExpired));
        assert!(get_valid(&cloud).await.is_none());
        assert!(load().is_none());
    }
}
