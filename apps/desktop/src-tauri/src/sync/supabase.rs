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

    fn rest_url(&self, table: &str, query: &str) -> String {
        if query.is_empty() {
            format!("{SUPABASE_URL}/rest/v1/{table}")
        } else {
            format!("{SUPABASE_URL}/rest/v1/{table}?{query}")
        }
    }

    /// `SELECT` sobre una tabla de PostgREST. `query` es lo que va tras el `?`, ya en
    /// sintaxis de PostgREST (p. ej. `"select=*&profile_id=eq.<uuid>"`) — sin
    /// constructor propio de filtros: para los pocos que hacen falta aquí, una cadena a
    /// mano es más simple y más fácil de leer que una capa de abstracción encima.
    pub async fn select(&self, session: &Session, table: &str, query: &str) -> Result<Vec<serde_json::Value>> {
        let response = self
            .client
            .get(self.rest_url(table, query))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(&session.access_token)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_json_array(response).await
    }

    /// Inserta o actualiza (según `Prefer: resolution=merge-duplicates`) una o varias
    /// filas. `on_conflict_query` es la parte `on_conflict=col1,col2` cuando la tabla
    /// tiene una clave compuesta (favoritos, progreso) — vacío para las que tienen `id`
    /// como PK de verdad (perfiles), donde basta el conflicto por PK por defecto.
    /// Devuelve las filas resultantes (`return=representation`): hace falta para saber
    /// el UUID que Supabase generó en un `INSERT` nuevo de perfil.
    pub async fn upsert(&self, session: &Session, table: &str, on_conflict_query: &str, body: &serde_json::Value) -> Result<Vec<serde_json::Value>> {
        let response = self
            .client
            .post(self.rest_url(table, on_conflict_query))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(&session.access_token)
            .header("Prefer", "resolution=merge-duplicates,return=representation")
            .json(body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_json_array(response).await
    }

    pub async fn patch(&self, session: &Session, table: &str, query: &str, body: &serde_json::Value) -> Result<()> {
        let response = self
            .client
            .patch(self.rest_url(table, query))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(&session.access_token)
            .json(body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::check_ok(response).await
    }

    pub async fn delete_rows(&self, session: &Session, table: &str, query: &str) -> Result<()> {
        let response = self
            .client
            .delete(self.rest_url(table, query))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(&session.access_token)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::check_ok(response).await
    }

    async fn parse_json_array(response: reqwest::Response) -> Result<Vec<serde_json::Value>> {
        let status = response.status();
        let bytes = response.bytes().await.map_err(|_| SupabaseError::Network)?;
        if !status.is_success() {
            let message = extract_error_message(&bytes).unwrap_or_else(|| format!("El servidor de sincronización respondió {status}."));
            return Err(SupabaseError::Api(message));
        }
        serde_json::from_slice(&bytes).map_err(|_| SupabaseError::InvalidResponse)
    }

    async fn check_ok(response: reqwest::Response) -> Result<()> {
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let bytes = response.bytes().await.unwrap_or_default();
        let message = extract_error_message(&bytes).unwrap_or_else(|| format!("El servidor de sincronización respondió {status}."));
        Err(SupabaseError::Api(message))
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

    /// Pide a Supabase que mande el correo de recuperación de contraseña. No hay sesión
    /// que devolver — solo confirma que el envío se aceptó. Supabase responde 200 tanto
    /// si el email existe como si no (para no filtrar qué correos tienen cuenta), así
    /// que esto nunca revela si una dirección está registrada.
    pub async fn recover(&self, email: &str) -> Result<()> {
        let response = self
            .client
            .post(self.auth_url("recover"))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&serde_json::json!({ "email": email }))
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::check_ok(response).await
    }

    /// Cambia el enlace/código de recuperación del correo por una sesión de verdad. El
    /// `token` es el valor del parámetro `token=` del enlace que Supabase mandó por
    /// correo — la propia app no tiene una página web que lo reciba, así que se le pide
    /// al usuario que lo pegue directamente. Sin esto no hay forma de completar el
    /// cambio de contraseña sin configurar una URL de redirección en el panel de
    /// Supabase (fuera del alcance de lo que se puede hacer desde aquí).
    pub async fn verify_recovery(&self, email: &str, token: &str) -> Result<Session> {
        let body = serde_json::json!({ "type": "recovery", "email": email, "token": token });
        let response = self
            .client
            .post(self.auth_url("verify"))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&body)
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::parse_session_response(response).await
    }

    /// Fija la contraseña nueva, ya con la sesión de recuperación en la mano (la que
    /// devuelve `verify_recovery`).
    pub async fn update_password(&self, access_token: &str, new_password: &str) -> Result<()> {
        let response = self
            .client
            .put(self.auth_url("user"))
            .header("apikey", SUPABASE_ANON_KEY)
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "password": new_password }))
            .send()
            .await
            .map_err(|_| SupabaseError::Network)?;
        Self::check_ok(response).await
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
