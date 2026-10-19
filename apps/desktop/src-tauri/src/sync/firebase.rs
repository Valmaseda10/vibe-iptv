//! Firebase como nube de la app (F11): cuentas por la API REST de Identity Toolkit y
//! datos por la API REST de Firestore. Sin SDK: no hay uno oficial para Rust, y para las
//! ocho llamadas que hacen falta un cliente propio sobre `reqwest` (ya dependencia) es
//! más simple que cualquier capa intermedia. Formas de respuesta comprobadas contra el
//! proyecto real, ver `docs/CLOUD-SYNC.md`.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use super::backend::{
    Changes, CloudBackend, CloudError, Collection, RemoteDoc, RemoteFavorite, RemotePlayback, RemoteProfile, Result, Session,
};
use crate::util::civil_time::{epoch_to_rfc3339, rfc3339_to_epoch};

/// Identificadores públicos del proyecto de Firebase. No son un secreto de usuario (la
/// regla R5 de `CLAUDE.md` es sobre credenciales de proveedor IPTV): la API key web de
/// Firebase solo identifica el proyecto y está pensada para ir dentro de los clientes.
/// Lo que protege los datos son las reglas de `infra/firebase/firestore.rules` (cada
/// cuenta solo lee y escribe bajo su `users/{uid}`), el mismo modelo que tenía la clave
/// `anon` de Supabase.
const PROJECT_ID: &str = "vibe-iptv-sync";
const API_KEY: &str = "AIzaSyAKUqMEg5Ii41AUoth_rg07k3dgVMchUtk";

/// Documentos por página al bajar cambios, y tope de páginas por bajada (30.000
/// documentos: muy por encima de lo que acumula una familia, y evita un bucle sin fin
/// si el servidor devolviera siempre lo mismo).
const PAGE_SIZE: usize = 300;
const MAX_PAGES: usize = 100;

const SERVER_TS_FIELD: &str = "server_updated_at";

pub struct FirebaseConfig {
    pub api_key: String,
    pub project_id: String,
    pub identity_base: String,
    pub token_base: String,
    pub firestore_base: String,
}

impl FirebaseConfig {
    pub fn production() -> Self {
        Self {
            api_key: API_KEY.to_string(),
            project_id: PROJECT_ID.to_string(),
            identity_base: "https://identitytoolkit.googleapis.com/v1".to_string(),
            token_base: "https://securetoken.googleapis.com/v1".to_string(),
            firestore_base: "https://firestore.googleapis.com/v1".to_string(),
        }
    }
}

pub struct FirebaseBackend {
    client: reqwest::Client,
    config: FirebaseConfig,
}

impl FirebaseBackend {
    pub fn new(config: FirebaseConfig) -> Self {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_default();
        Self { client, config }
    }

    fn documents_root(&self) -> String {
        format!("projects/{}/databases/(default)/documents", self.config.project_id)
    }

    fn doc_name(&self, user_id: &str, doc: &RemoteDoc) -> String {
        format!("{}/users/{user_id}/{}/{}", self.documents_root(), doc.collection().as_str(), doc.doc_id())
    }

    async fn post_json(&self, url: String, bearer: Option<&str>, body: &Value) -> Result<(StatusCode, Value)> {
        let mut request = self.client.post(url).json(body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| CloudError::Network)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|_| CloudError::Network)?;
        // Un cuerpo vacío o que no es JSON solo es un problema si además la petición
        // falló; un 5xx de un balanceador puede venir en HTML.
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok((status, value))
    }

    async fn identity(&self, method: &str, body: Value) -> Result<Value> {
        let url = format!("{}/accounts:{method}?key={}", self.config.identity_base, self.config.api_key);
        let (status, value) = self.post_json(url, None, &body).await?;
        if status.is_success() {
            Ok(value)
        } else {
            Err(auth_error(status, &value))
        }
    }
}

#[async_trait]
impl CloudBackend for FirebaseBackend {
    async fn sign_up(&self, email: &str, password: &str) -> Result<Session> {
        let value = self.identity("signUp", json!({ "email": email, "password": password, "returnSecureToken": true })).await?;
        session_from_identity(&value, email)
    }

    async fn sign_in(&self, email: &str, password: &str) -> Result<Session> {
        let value = self.identity("signInWithPassword", json!({ "email": email, "password": password, "returnSecureToken": true })).await?;
        session_from_identity(&value, email)
    }

    async fn refresh(&self, session: &Session) -> Result<Session> {
        let url = format!("{}/token?key={}", self.config.token_base, self.config.api_key);
        let body = json!({ "grant_type": "refresh_token", "refresh_token": session.refresh_token });
        let (status, value) = self.post_json(url, None, &body).await?;
        if !status.is_success() {
            return Err(match auth_error(status, &value) {
                // Cualquier rechazo del token de refresco con un 4xx significa que esta
                // sesión ya no vale (revocada, contraseña cambiada en otro sitio, cuenta
                // borrada): no tiene sentido reintentarlo.
                CloudError::Rejected(_) => CloudError::SessionExpired,
                other => other,
            });
        }
        let (Some(access_token), Some(refresh_token), Some(expires_in)) =
            (str_field(&value, "id_token"), str_field(&value, "refresh_token"), int_field(&value, "expires_in"))
        else {
            return Err(CloudError::InvalidResponse);
        };
        Ok(Session {
            access_token,
            refresh_token,
            user_id: str_field(&value, "user_id").unwrap_or_else(|| session.user_id.clone()),
            email: session.email.clone(),
            expires_at: crate::db::now_epoch() + expires_in,
        })
    }

    async fn send_password_reset(&self, email: &str) -> Result<()> {
        match self.identity("sendOobCode", json!({ "requestType": "PASSWORD_RESET", "email": email })).await {
            Ok(_) => Ok(()),
            // Proyectos sin protección contra enumeración de correos responden esto para
            // un correo sin cuenta; se trata igual que un envío correcto para no revelar
            // qué correos están registrados.
            Err(CloudError::Rejected(message)) if message == auth_message("EMAIL_NOT_FOUND") => Ok(()),
            Err(err) => Err(err),
        }
    }

    async fn confirm_password_reset(&self, code: &str, new_password: &str) -> Result<()> {
        let code = extract_oob_code(code);
        self.identity("resetPassword", json!({ "oobCode": code, "newPassword": new_password })).await.map(|_| ())
    }

    async fn change_password(&self, session: &Session, new_password: &str) -> Result<Session> {
        let value = self
            .identity("update", json!({ "idToken": session.access_token, "password": new_password, "returnSecureToken": true }))
            .await?;
        session_from_identity(&value, &session.email)
    }

    async fn write(&self, session: &Session, docs: &[RemoteDoc]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }
        let writes: Vec<Value> = docs
            .iter()
            .map(|doc| {
                json!({
                    "update": { "name": self.doc_name(&session.user_id, doc), "fields": encode_fields(doc) },
                    // Marca de tiempo del servidor, no del cliente: es la que ordena la
                    // bajada incremental, y los relojes de dos dispositivos nunca coinciden.
                    "updateTransforms": [{ "fieldPath": SERVER_TS_FIELD, "setToServerValue": "REQUEST_TIME" }],
                })
            })
            .collect();
        let url = format!("{}/{}:commit", self.config.firestore_base, self.documents_root());
        let (status, value) = self.post_json(url, Some(&session.access_token), &json!({ "writes": writes })).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(data_error(status, &value))
        }
    }

    async fn changes_since(&self, session: &Session, collection: Collection, since: i64) -> Result<Changes> {
        let url = format!("{}/{}/users/{}:runQuery", self.config.firestore_base, self.documents_root(), session.user_id);
        let mut changes = Changes::default();
        let mut after: Option<(String, String)> = None;

        for _ in 0..MAX_PAGES {
            let mut query = json!({
                "from": [{ "collectionId": collection.as_str() }],
                "where": { "fieldFilter": {
                    "field": { "fieldPath": SERVER_TS_FIELD },
                    "op": "GREATER_THAN",
                    "value": { "timestampValue": epoch_to_rfc3339(since) },
                }},
                "orderBy": [
                    { "field": { "fieldPath": SERVER_TS_FIELD }, "direction": "ASCENDING" },
                    { "field": { "fieldPath": "__name__" }, "direction": "ASCENDING" },
                ],
                "limit": PAGE_SIZE,
            });
            if let Some((ts, name)) = &after {
                query["startAt"] = json!({ "values": [{ "timestampValue": ts }, { "referenceValue": name }], "before": false });
            }

            let (status, value) = self.post_json(url.clone(), Some(&session.access_token), &json!({ "structuredQuery": query })).await?;
            if !status.is_success() {
                return Err(data_error(status, &value));
            }
            let items = value.as_array().ok_or(CloudError::InvalidResponse)?;

            let mut in_page = 0;
            for item in items {
                let Some(document) = item.get("document") else { continue };
                in_page += 1;
                let (Some(name), Some(server_ts)) = (
                    document.get("name").and_then(Value::as_str),
                    document.get("fields").and_then(|f| f.get(SERVER_TS_FIELD)).and_then(|v| v.get("timestampValue")).and_then(Value::as_str),
                ) else {
                    continue;
                };
                if let Some(epoch) = rfc3339_to_epoch(server_ts) {
                    changes.max_server_ts = Some(changes.max_server_ts.map_or(epoch, |m| m.max(epoch)));
                }
                after = Some((server_ts.to_string(), name.to_string()));
                // Un documento con una forma que no se entiende se salta, no tumba la
                // bajada entera (mismo criterio que los `*Wire` de Xtream).
                if let Some(doc) = decode_doc(collection, name, document) {
                    changes.docs.push(doc);
                }
            }
            if in_page < PAGE_SIZE {
                break;
            }
        }
        Ok(changes)
    }
}

// ─────────────── Sesión y errores de cuentas ───────────────

fn session_from_identity(value: &Value, fallback_email: &str) -> Result<Session> {
    let (Some(access_token), Some(refresh_token), Some(user_id), Some(expires_in)) = (
        str_field(value, "idToken"),
        str_field(value, "refreshToken"),
        str_field(value, "localId"),
        int_field(value, "expiresIn"),
    ) else {
        return Err(CloudError::InvalidResponse);
    };
    Ok(Session {
        access_token,
        refresh_token,
        user_id,
        email: str_field(value, "email").unwrap_or_else(|| fallback_email.to_string()),
        expires_at: crate::db::now_epoch() + expires_in,
    })
}

/// Código de error de Identity Toolkit sin la explicación en inglés que a veces le
/// sigue (`"WEAK_PASSWORD : Password should be at least 6 characters"`).
fn error_code(value: &Value) -> Option<String> {
    let message = value.get("error")?.get("message")?.as_str()?;
    Some(message.split(" : ").next().unwrap_or(message).trim().to_string())
}

fn auth_error(status: StatusCode, value: &Value) -> CloudError {
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return CloudError::Unavailable;
    }
    match error_code(value) {
        Some(code) => CloudError::Rejected(auth_message(&code)),
        None => CloudError::InvalidResponse,
    }
}

/// Mensaje en español, para enseñarse tal cual, de cada código de Identity Toolkit que
/// puede devolver un flujo de esta app.
fn auth_message(code: &str) -> String {
    match code {
        "EMAIL_EXISTS" => "Ya existe una cuenta con ese correo. Inicia sesión.",
        "INVALID_LOGIN_CREDENTIALS" | "EMAIL_NOT_FOUND" | "INVALID_PASSWORD" => "Correo o contraseña incorrectos.",
        "USER_DISABLED" => "Esta cuenta está desactivada.",
        "WEAK_PASSWORD" => "La contraseña es demasiado débil: usa al menos 6 caracteres.",
        "INVALID_EMAIL" | "MISSING_EMAIL" => "El correo no tiene un formato válido.",
        "MISSING_PASSWORD" => "Escribe una contraseña.",
        "TOO_MANY_ATTEMPTS_TRY_LATER" => "Demasiados intentos seguidos. Espera unos minutos y vuelve a probar.",
        "INVALID_OOB_CODE" | "EXPIRED_OOB_CODE" | "MISSING_OOB_CODE" => {
            "El enlace de recuperación no es válido o ya caducó. Pide otro correo."
        }
        "CREDENTIAL_TOO_OLD_LOGIN_AGAIN" | "TOKEN_EXPIRED" | "INVALID_ID_TOKEN" => {
            "Por seguridad, cierra sesión y vuelve a entrar antes de cambiar la contraseña."
        }
        "OPERATION_NOT_ALLOWED" | "CONFIGURATION_NOT_FOUND" | "PASSWORD_LOGIN_DISABLED" => {
            "El servidor de cuentas no tiene activado el inicio de sesión con correo."
        }
        other => return format!("El servidor de cuentas rechazó la petición ({other})."),
    }
    .to_string()
}

fn data_error(status: StatusCode, value: &Value) -> CloudError {
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return CloudError::Unavailable;
    }
    // runQuery devuelve los errores dentro de un array.
    let error = value.get("error").or_else(|| value.get(0).and_then(|v| v.get("error")));
    let detail = error.and_then(|e| e.get("status")).and_then(Value::as_str).unwrap_or("desconocido");
    CloudError::Rejected(format!("La nube rechazó la sincronización ({detail})."))
}

/// El código del correo de recuperación, tanto si se pega solo como si se pega el
/// enlace entero (`...?mode=resetPassword&oobCode=XYZ&apiKey=...`).
fn extract_oob_code(code_or_link: &str) -> &str {
    let trimmed = code_or_link.trim();
    match trimmed.find("oobCode=") {
        Some(pos) => {
            let after = &trimmed[pos + "oobCode=".len()..];
            after.split('&').next().unwrap_or(after)
        }
        None => trimmed,
    }
}

// ─────────────── Codificación de documentos de Firestore ───────────────

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Firebase manda algunos enteros como cadena (`"expiresIn": "3600"`).
fn int_field(value: &Value, key: &str) -> Option<i64> {
    match value.get(key)? {
        Value::String(s) => s.parse().ok(),
        other => other.as_i64(),
    }
}

fn fs_string(value: &str) -> Value {
    json!({ "stringValue": value })
}

fn fs_opt_string(value: Option<&str>) -> Value {
    value.map_or(json!({ "nullValue": null }), fs_string)
}

/// Firestore representa los enteros de 64 bits como cadena en JSON.
fn fs_int(value: i64) -> Value {
    json!({ "integerValue": value.to_string() })
}

fn fs_opt_int(value: Option<i64>) -> Value {
    value.map_or(json!({ "nullValue": null }), fs_int)
}

fn fs_bool(value: bool) -> Value {
    json!({ "booleanValue": value })
}

fn encode_fields(doc: &RemoteDoc) -> Value {
    let mut fields = Map::new();
    match doc {
        RemoteDoc::Profile(p) => {
            fields.insert("name".into(), fs_string(&p.name));
            fields.insert("avatar".into(), fs_opt_string(p.avatar.as_deref()));
            fields.insert("adult_ok".into(), fs_bool(p.adult_ok));
            fields.insert("max_age".into(), p.max_age.map_or(json!({ "nullValue": null }), fs_int));
            fields.insert("pin_hash".into(), fs_opt_string(p.pin_hash.as_deref()));
            fields.insert("updated_at".into(), fs_int(p.updated_at));
            fields.insert("deleted".into(), fs_bool(p.deleted));
        }
        RemoteDoc::Favorite(f) => {
            fields.insert("profile_id".into(), fs_string(&f.profile_id));
            fields.insert("item_kind".into(), fs_string(&f.item_kind));
            fields.insert("item_key".into(), fs_string(&f.item_key));
            fields.insert("added_at".into(), fs_int(f.added_at));
            fields.insert("updated_at".into(), fs_int(f.updated_at));
            fields.insert("deleted".into(), fs_bool(f.deleted));
        }
        RemoteDoc::Playback(p) => {
            fields.insert("profile_id".into(), fs_string(&p.profile_id));
            fields.insert("item_kind".into(), fs_string(&p.item_kind));
            fields.insert("item_key".into(), fs_string(&p.item_key));
            fields.insert("position_s".into(), fs_int(p.position_s));
            fields.insert("duration_s".into(), fs_opt_int(p.duration_s));
            fields.insert("finished".into(), fs_bool(p.finished));
            fields.insert("played_at".into(), fs_int(p.played_at));
            fields.insert("updated_at".into(), fs_int(p.updated_at));
            fields.insert("deleted".into(), fs_bool(p.deleted));
        }
    }
    Value::Object(fields)
}

struct Fields<'a>(&'a Value);

impl Fields<'_> {
    fn raw(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    fn string(&self, key: &str) -> Option<String> {
        self.raw(key)?.get("stringValue")?.as_str().map(str::to_string)
    }

    fn int(&self, key: &str) -> Option<i64> {
        let value = self.raw(key)?;
        match value.get("integerValue") {
            Some(Value::String(s)) => s.parse().ok(),
            Some(other) => other.as_i64(),
            // Otro cliente (iOS) podría guardar un número como double.
            None => value.get("doubleValue").and_then(Value::as_f64).map(|f| f as i64),
        }
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.raw(key)?.get("booleanValue")?.as_bool()
    }
}

fn decode_doc(collection: Collection, name: &str, document: &Value) -> Option<RemoteDoc> {
    let fields = Fields(document.get("fields")?);
    let deleted = fields.bool("deleted").unwrap_or(false);
    let updated_at = fields.int("updated_at").unwrap_or(0);
    Some(match collection {
        Collection::Profiles => RemoteDoc::Profile(RemoteProfile {
            id: name.rsplit('/').next()?.to_string(),
            name: fields.string("name").unwrap_or_default(),
            avatar: fields.string("avatar"),
            adult_ok: fields.bool("adult_ok").unwrap_or(false),
            max_age: fields.int("max_age"),
            pin_hash: fields.string("pin_hash"),
            updated_at,
            deleted,
        }),
        Collection::Favorites => RemoteDoc::Favorite(RemoteFavorite {
            profile_id: fields.string("profile_id")?,
            item_kind: fields.string("item_kind")?,
            item_key: fields.string("item_key")?,
            added_at: fields.int("added_at").unwrap_or(updated_at),
            updated_at,
            deleted,
        }),
        Collection::Playback => RemoteDoc::Playback(RemotePlayback {
            profile_id: fields.string("profile_id")?,
            item_kind: fields.string("item_kind")?,
            item_key: fields.string("item_key")?,
            position_s: fields.int("position_s").unwrap_or(0),
            duration_s: fields.int("duration_s"),
            finished: fields.bool("finished").unwrap_or(false),
            played_at: fields.int("played_at").unwrap_or(updated_at),
            updated_at,
            deleted,
        }),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::StatusCode as AxumStatus;
    use axum::routing::post;
    use axum::{Json, Router};

    use super::*;

    /// Lo que el falso servidor de Firebase ha recibido, para que el test lo compruebe.
    #[derive(Default)]
    struct Seen {
        bodies: Vec<(String, Value)>,
        query_calls: usize,
    }

    type Shared = Arc<Mutex<Seen>>;

    /// Respuestas con la forma exacta que dio el proyecto real (ver `docs/CLOUD-SYNC.md`),
    /// servidas por un axum local: mismo criterio que los tests de `sources::xtream`, sin
    /// librería de mocking HTTP.
    async fn spawn_fake_firebase() -> (FirebaseBackend, Shared) {
        let seen: Shared = Arc::default();
        let app = Router::new()
            .route(
                "/identity/*method",
                post(|State(seen): State<Shared>, axum::extract::Path(method): axum::extract::Path<String>, Json(body): Json<Value>| async move {
                    seen.lock().unwrap().bodies.push((method.clone(), body.clone()));
                    if method.starts_with("accounts:signInWithPassword") && body["password"] == "mala" {
                        return (AxumStatus::BAD_REQUEST, Json(json!({ "error": { "code": 400, "message": "INVALID_LOGIN_CREDENTIALS" } })));
                    }
                    if method.starts_with("accounts:signUp") && body["password"] == "12" {
                        return (
                            AxumStatus::BAD_REQUEST,
                            Json(json!({ "error": { "code": 400, "message": "WEAK_PASSWORD : Password should be at least 6 characters" } })),
                        );
                    }
                    if method.starts_with("accounts:resetPassword") {
                        return (AxumStatus::OK, Json(json!({ "email": "ana@example.com", "requestType": "PASSWORD_RESET" })));
                    }
                    (
                        AxumStatus::OK,
                        Json(json!({
                            "kind": "identitytoolkit#SignupNewUserResponse",
                            "idToken": "id-token",
                            "email": body["email"],
                            "refreshToken": "refresh-token",
                            "expiresIn": "3600",
                            "localId": "uid-1",
                        })),
                    )
                }),
            )
            .route(
                "/token/*rest",
                post(|Json(body): Json<Value>| async move {
                    if body["refresh_token"] == "revocado" {
                        return (AxumStatus::BAD_REQUEST, Json(json!({ "error": { "code": 400, "message": "INVALID_REFRESH_TOKEN", "status": "INVALID_ARGUMENT" } })));
                    }
                    if body["refresh_token"] == "caido" {
                        return (AxumStatus::SERVICE_UNAVAILABLE, Json(Value::Null));
                    }
                    (
                        AxumStatus::OK,
                        Json(json!({
                            "access_token": "nuevo", "expires_in": "3600", "token_type": "Bearer",
                            "refresh_token": "refresh-2", "id_token": "nuevo", "user_id": "uid-1", "project_id": "1"
                        })),
                    )
                }),
            )
            .route(
                "/firestore/*rest",
                post(|State(seen): State<Shared>, axum::extract::Path(rest): axum::extract::Path<String>, Json(body): Json<Value>| async move {
                    let mut seen = seen.lock().unwrap();
                    seen.bodies.push((rest.clone(), body.clone()));
                    if rest.ends_with(":commit") {
                        return Json(json!({ "writeResults": [{ "updateTime": "2026-10-15T17:48:23.208042Z" }], "commitTime": "2026-10-15T17:48:23.208042Z" }));
                    }
                    seen.query_calls += 1;
                    let doc = |id: &str, ts: &str| {
                        json!({ "document": {
                            "name": format!("projects/p/databases/(default)/documents/users/uid-1/profiles/{id}"),
                            "fields": {
                                "name": { "stringValue": format!("Perfil {id}") },
                                "avatar": { "nullValue": null },
                                "adult_ok": { "booleanValue": true },
                                "updated_at": { "integerValue": "1700000000" },
                                "deleted": { "booleanValue": false },
                                "server_updated_at": { "timestampValue": ts },
                            },
                        }, "readTime": "2026-10-15T17:48:23.684498Z" })
                    };
                    if seen.query_calls == 1 {
                        // Página llena: obliga a pedir la siguiente con el cursor.
                        let docs: Vec<Value> = (0..PAGE_SIZE).map(|i| doc(&format!("a{i:03}"), "2023-11-14T22:13:20.180Z")).collect();
                        Json(Value::Array(docs))
                    } else {
                        Json(json!([doc("b", "2023-11-14T22:15:00.500Z"), { "readTime": "2026-10-15T17:48:24Z" }]))
                    }
                }),
            )
            .with_state(seen.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let backend = FirebaseBackend::new(FirebaseConfig {
            api_key: "key".into(),
            project_id: "p".into(),
            identity_base: format!("{base}/identity"),
            token_base: format!("{base}/token"),
            firestore_base: format!("{base}/firestore"),
        });
        (backend, seen)
    }

    fn session() -> Session {
        Session { access_token: "id-token".into(), refresh_token: "refresh-token".into(), user_id: "uid-1".into(), email: "ana@example.com".into(), expires_at: 0 }
    }

    #[tokio::test]
    async fn sign_in_parses_the_real_response_shape() {
        let (backend, _) = spawn_fake_firebase().await;
        let session = backend.sign_in("ana@example.com", "buena").await.unwrap();
        assert_eq!(session.user_id, "uid-1");
        assert_eq!(session.email, "ana@example.com");
        assert!(session.expires_at > crate::db::now_epoch() + 3000, "expiresIn llega como cadena");
    }

    #[tokio::test]
    async fn auth_errors_become_spanish_messages() {
        let (backend, _) = spawn_fake_firebase().await;
        assert_eq!(backend.sign_in("ana@example.com", "mala").await, Err(CloudError::Rejected("Correo o contraseña incorrectos.".into())));
        assert_eq!(
            backend.sign_up("ana@example.com", "12").await,
            Err(CloudError::Rejected("La contraseña es demasiado débil: usa al menos 6 caracteres.".into()))
        );
    }

    #[tokio::test]
    async fn a_revoked_refresh_token_expires_the_session_but_an_outage_does_not() {
        let (backend, _) = spawn_fake_firebase().await;
        let refreshed = backend.refresh(&session()).await.unwrap();
        assert_eq!(refreshed.access_token, "nuevo");
        assert_eq!(refreshed.email, "ana@example.com", "el refresco no trae el correo; se conserva");

        let revoked = Session { refresh_token: "revocado".into(), ..session() };
        assert_eq!(backend.refresh(&revoked).await, Err(CloudError::SessionExpired));

        let outage = Session { refresh_token: "caido".into(), ..session() };
        let err = backend.refresh(&outage).await.unwrap_err();
        assert!(err.is_transient(), "un 503 no puede cerrar la sesión: {err:?}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let backend = FirebaseBackend::new(FirebaseConfig {
            api_key: "key".into(),
            project_id: "p".into(),
            identity_base: "http://127.0.0.1:1".into(),
            token_base: "http://127.0.0.1:1".into(),
            firestore_base: "http://127.0.0.1:1".into(),
        });
        assert_eq!(backend.sign_in("a@b.c", "x").await, Err(CloudError::Network));
        assert_eq!(backend.refresh(&session()).await, Err(CloudError::Network));
    }

    #[tokio::test]
    async fn password_reset_accepts_the_whole_link_from_the_email() {
        let (backend, seen) = spawn_fake_firebase().await;
        backend
            .confirm_password_reset("https://vibe-iptv-sync.firebaseapp.com/__/auth/action?mode=resetPassword&oobCode=ABC123&apiKey=k&lang=es", "nueva-clave")
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        let (_, body) = seen.bodies.iter().find(|(m, _)| m.starts_with("accounts:resetPassword")).unwrap();
        assert_eq!(body["oobCode"], "ABC123");
    }

    #[tokio::test]
    async fn write_sends_typed_fields_and_a_server_timestamp() {
        let (backend, seen) = spawn_fake_firebase().await;
        let doc = RemoteDoc::Playback(RemotePlayback {
            profile_id: "p1".into(),
            item_kind: "movie".into(),
            item_key: "matrix-1999".into(),
            position_s: 600,
            duration_s: None,
            finished: false,
            played_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            deleted: false,
        });
        backend.write(&session(), std::slice::from_ref(&doc)).await.unwrap();

        let seen = seen.lock().unwrap();
        let (path, body) = seen.bodies.iter().find(|(p, _)| p.ends_with(":commit")).unwrap();
        assert_eq!(path, "projects/p/databases/(default)/documents:commit");
        let write = &body["writes"][0];
        assert_eq!(write["update"]["name"], format!("projects/p/databases/(default)/documents/users/uid-1/playback/{}", doc.doc_id()));
        assert_eq!(write["update"]["fields"]["position_s"], json!({ "integerValue": "600" }));
        assert_eq!(write["update"]["fields"]["duration_s"], json!({ "nullValue": null }));
        assert_eq!(write["updateTransforms"][0]["setToServerValue"], "REQUEST_TIME");
    }

    #[tokio::test]
    async fn changes_since_pages_with_a_cursor_and_decodes_documents() {
        let (backend, seen) = spawn_fake_firebase().await;
        let changes = backend.changes_since(&session(), Collection::Profiles, 1_000).await.unwrap();

        assert_eq!(changes.docs.len(), PAGE_SIZE + 1);
        let RemoteDoc::Profile(last) = changes.docs.last().unwrap() else { panic!("se esperaba un perfil") };
        assert_eq!(last.id, "b");
        assert!(last.adult_ok);
        assert_eq!(changes.max_server_ts, Some(1_700_000_100));

        let seen = seen.lock().unwrap();
        let queries: Vec<&Value> = seen.bodies.iter().filter(|(p, _)| p.ends_with(":runQuery")).map(|(_, b)| b).collect();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0]["structuredQuery"]["where"]["fieldFilter"]["value"]["timestampValue"], "1970-01-01T00:16:40Z");
        assert!(queries[0]["structuredQuery"].get("startAt").is_none());
        let cursor = &queries[1]["structuredQuery"]["startAt"]["values"];
        assert_eq!(cursor[0]["timestampValue"], "2023-11-14T22:13:20.180Z", "el cursor reutiliza la marca exacta del servidor");
        assert!(cursor[1]["referenceValue"].as_str().unwrap().ends_with("/profiles/a299"));
    }

    /// Verificación manual contra el proyecto real, no se ejecuta en `cargo test`. Las
    /// credenciales de una cuenta de pruebas llegan por entorno, nunca del repositorio:
    /// `VIBE_QA_EMAIL=... VIBE_QA_PASSWORD=... cargo test live_ -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_roundtrip_against_the_real_project() {
        let (Ok(email), Ok(password)) = (std::env::var("VIBE_QA_EMAIL"), std::env::var("VIBE_QA_PASSWORD")) else {
            panic!("faltan VIBE_QA_EMAIL / VIBE_QA_PASSWORD");
        };
        let backend = FirebaseBackend::new(FirebaseConfig::production());
        let session = backend.sign_in(&email, &password).await.unwrap();
        let refreshed = backend.refresh(&session).await.unwrap();
        assert_eq!(refreshed.user_id, session.user_id);

        let before = crate::db::now_epoch() - 5;
        let doc = RemoteDoc::Favorite(RemoteFavorite {
            profile_id: "live-test".into(),
            item_kind: "movie".into(),
            item_key: format!("live-test-{before}"),
            added_at: before,
            updated_at: before,
            deleted: false,
        });
        backend.write(&refreshed, std::slice::from_ref(&doc)).await.unwrap();
        let changes = backend.changes_since(&refreshed, Collection::Favorites, before - 60).await.unwrap();
        assert!(changes.docs.contains(&doc), "lo escrito vuelve en la bajada incremental");
        assert!(changes.max_server_ts.unwrap() >= before);

        // Lo deja como borrado lógico, igual que haría la app.
        let RemoteDoc::Favorite(fav) = doc else { unreachable!() };
        backend.write(&refreshed, &[RemoteDoc::Favorite(RemoteFavorite { deleted: true, ..fav })]).await.unwrap();

        assert_eq!(backend.sign_in(&email, "contraseña-equivocada").await, Err(CloudError::Rejected("Correo o contraseña incorrectos.".into())));
    }

    #[test]
    fn decode_skips_documents_missing_required_fields() {
        let broken = json!({ "fields": { "item_kind": { "stringValue": "movie" } } });
        assert!(decode_doc(Collection::Favorites, "x/favorites/1", &broken).is_none());
    }
}
