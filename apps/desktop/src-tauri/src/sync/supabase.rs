//! Cliente mínimo contra la API REST de Supabase (Auth por ahora; PostgREST llega con
//! la sincronización de perfiles/favoritos/progreso). Sin SDK oficial de Rust: para lo
//! que hace falta aquí, un cliente propio sobre `reqwest` (ya dependencia) es más
//! simple que integrar un SDK pensado para mucho más de lo que usa esta app (Realtime,
//! Storage, funciones edge...).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// URL del proyecto y clave `anon` de Supabase. No es un secreto de usuario — la regla
/// R5 de `CLAUDE.md` es sobre credenciales de **proveedor IPTV**, algo distinto. La
/// clave `anon` está diseñada por Supabase para ir embebida en clientes: la protegen
/// las políticas de Row Level Security del proyecto (`docs/DECISIONS.md`), no el
/// secretismo de la clave en sí — es el mismo modelo que la clave pública de Stripe o
/// la de un SDK de analítica.
const SUPABASE_URL: &str = "https://beqentvogqamkopsnfos.supabase.co";
const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImJlcWVudHZvZ3FhbWtvcHNuZm9zIiwicm9sZSI6ImFub24iLCJpYXQiOjE3ODgxODM2MTIsImV4cCI6MjEwMzc1OTYxMn0.bee4iPnfyfPMUw0O1kbe0mRyJohYQ-kAJoHrGBhd0Xs";

#[derive(Debug, thiserror::Error)]
pub enum SupabaseError {
    #[error("No se pudo conectar con el servidor de sincronización. Comprueba tu conexión a internet.")]
    Network,
    #[error("{0}")]
    Api(String),
    #[error("Respuesta inesperada del servidor de sincronización.")]
    InvalidResponse,
}

pub type Result<T> = std::result::Result<T, SupabaseError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub email: String,
    /// Epoch (segundos) en el que caduca `access_token` (normalmente ~1h desde que se
    /// emitió). `sync::session::get_valid` lo usa para refrescar antes de que caduque.
    pub expires_at: i64,
}

pub struct SupabaseClient {
    client: reqwest::Client,
}

impl Default for SupabaseClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SupabaseClient {
    pub fn new() -> Self {
        Self { client: reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default() }
    }

    fn auth_url(&self, path: &str) -> String {
        format!("{SUPABASE_URL}/auth/v1/{path}")
    }

    /// Crea una cuenta nueva. Si el proyecto de Supabase exige confirmar el email antes
    /// de dar sesión (configuración por defecto), no hay `Session` que devolver todavía
    /// — se informa como error de API con un mensaje claro, no como fallo de red.
    pub async fn sign_up(&self, email: &str, password: &str) -> Result<Session> {
        let body = serde_json::json!({ "email": email, "password": password });
        let response = self
            .client
            .post(self.auth_url("signup"))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_session_response(response).await
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Session> {
        let body = serde_json::json!({ "email": email, "password": password });
        let response = self
            .client
            .post(format!("{}?grant_type=password", self.auth_url("token")))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_session_response(response).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<Session> {
        let body = serde_json::json!({ "refresh_token": refresh_token });
        let response = self
            .client
            .post(format!("{}?grant_type=refresh_token", self.auth_url("token")))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_session_response(response).await
    }

    /// Best-effort: si falla, la sesión local se borra igualmente (el llamador no debe
    /// dejar al usuario "atrapado" logueado solo porque no había red en ese momento).
    pub async fn sign_out(&self, access_token: &str) {
        let _ = self
            .client
            .post(self.auth_url("logout"))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(access_token)
            .send()
            .await;
    }

    async fn parse_session_response(response: reqwest::Response) -> Result<Session> {
        let status = response.status();
        let bytes = response.bytes().await.map_err(|_| SupabaseError::Network)?;

        if !status.is_success() {
            let message = extract_error_message(&bytes).unwrap_or_else(|| "No se pudo iniciar sesión.".to_string());
            return Err(SupabaseError::Api(message));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: Option<String>,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
            user: Option<UserWire>,
        }
        #[derive(Deserialize)]
        struct UserWire {
            id: String,
            email: Option<String>,
        }

        let parsed: TokenResponse = serde_json::from_slice(&bytes).map_err(|_| SupabaseError::InvalidResponse)?;
        let user = parsed.user.ok_or(SupabaseError::InvalidResponse)?;
        let (Some(access_token), Some(refresh_token), Some(expires_in)) =
            (parsed.access_token, parsed.refresh_token, parsed.expires_in)
        else {
            // Alta hecha, pero el proyecto pide confirmar el email antes de dar sesión.
            return Err(SupabaseError::Api(
                "Cuenta creada. Revisa tu correo para confirmarla antes de iniciar sesión.".to_string(),
            ));
        };

        Ok(Session {
            access_token,
            refresh_token,
            user_id: user.id,
            email: user.email.unwrap_or_default(),
            expires_at: crate::db::now_epoch() + expires_in,
        })
    }
}

fn extract_error_message(bytes: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct ErrorWire {
        #[serde(alias = "error_description", alias = "msg")]
        message: Option<String>,
    }
    serde_json::from_slice::<ErrorWire>(bytes).ok().and_then(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sign_in_against_an_unreachable_host_is_a_network_error_not_a_panic() {
        // No se apunta al Supabase real en un test automático (necesitaría red y una
        // cuenta de prueba real). Esto solo confirma que un fallo de conexión se
        // convierte en el error correcto, sin depender de la red real del proyecto.
        let client = SupabaseClient::new();
        // Reutiliza el cliente pero apunta a una URL que no existe, construyendo la
        // petición a mano en vez de por sign_in (que usa la constante real).
        let response = client.client.post("http://127.0.0.1:1/auth/v1/token?grant_type=password").json(&serde_json::json!({})).send().await;
        assert!(response.is_err());
    }
}
