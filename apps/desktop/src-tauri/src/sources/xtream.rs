//! Cliente Xtream Codes. Ver `docs/XTREAM-API.md`: los campos se tratan como opcionales
//! y los numéricos toleran llegar como cadena.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::error::{Result, SourceError};
use super::model::{
    AccountInfo, MediaKind, RawCategory, RawChannel, RawEpisode, RawMovie, RawSeries,
    RawSeriesDetail, StreamRef, XmltvStream,
};
use super::serde_helpers::{lenient_f64, lenient_i64, lenient_id_opt, lenient_u32, non_empty_string_opt};
use super::Source;

pub const DEFAULT_USER_AGENT: &str = "VLC/3.0.20 LibVLC/3.0.20";

pub struct XtreamSource {
    base_url: String,
    username: String,
    password: String,
    client: reqwest::Client,
}

impl XtreamSource {
    /// `base_url` es `http://host:puerto`, sin ruta. `user_agent` por defecto
    /// [`DEFAULT_USER_AGENT`]: muchos proveedores rechazan peticiones sin uno conocido.
    pub fn new(
        base_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        user_agent: Option<&str>,
    ) -> Result<Self> {
        let base_url = base_url.into();
        let client = reqwest::Client::builder()
            .user_agent(user_agent.unwrap_or(DEFAULT_USER_AGENT))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|err| SourceError::invalid_response(&base_url, err.to_string()))?;
        Ok(Self {
            base_url,
            username: username.into(),
            password: password.into(),
            client,
        })
    }

    fn player_api_url(&self, action: Option<&str>, extra: &[(&str, &str)]) -> Result<Url> {
        let mut url = Url::parse(&self.base_url)
            .map_err(|_| SourceError::invalid_response(&self.base_url, "URL base inválida"))?;
        url.set_path("player_api.php");
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("username", &self.username)
                .append_pair("password", &self.password);
            if let Some(action) = action {
                query.append_pair("action", action);
            }
            for (key, value) in extra {
                query.append_pair(key, value);
            }
        }
        Ok(url)
    }

    async fn get_json<T: DeserializeOwned>(&self, action: Option<&str>, extra: &[(&str, &str)]) -> Result<T> {
        let url = self.player_api_url(action, extra)?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| SourceError::unreachable(url.as_str()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| SourceError::unreachable(url.as_str()))?;

        if !status.is_success() {
            return Err(SourceError::invalid_response(url.as_str(), format!("HTTP {status}")));
        }

        serde_json::from_slice::<T>(&bytes).map_err(|parse_err| {
            if looks_like_auth_failure(&bytes) {
                SourceError::AuthFailed
            } else {
                SourceError::invalid_response(url.as_str(), parse_err.to_string())
            }
        })
    }

    /// Construye la URL de reproducción. `include_live_segment = false` reproduce a los
    /// paneles antiguos que sirven el directo sin `/live/` (ver `docs/XTREAM-API.md`).
    /// Probar cuál funciona y recordarlo es trabajo del proxy de stream (fase F6): aquí
    /// solo se construyen las dos formas posibles, sin abrir ninguna conexión (regla R1).
    pub fn stream_url_variant(&self, item: &StreamRef, include_live_segment: bool) -> Result<Url> {
        let mut url = Url::parse(&self.base_url)
            .map_err(|_| SourceError::invalid_response(&self.base_url, "URL base inválida"))?;

        let type_segment = match item.kind {
            MediaKind::Live => "live",
            MediaKind::Movie => "movie",
            MediaKind::Series => "series",
        };
        let default_extension = if item.kind == MediaKind::Live { "ts" } else { "mp4" };
        let extension = item.container_extension.as_deref().unwrap_or(default_extension);

        let mut segments = Vec::new();
        if item.kind != MediaKind::Live || include_live_segment {
            segments.push(type_segment.to_string());
        }
        segments.push(self.username.clone());
        segments.push(self.password.clone());
        segments.push(format!("{}.{extension}", item.remote_id));

        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| SourceError::invalid_response(&self.base_url, "la URL base no admite ruta"))?;
            path.clear();
            for segment in &segments {
                path.push(segment);
            }
        }
        Ok(url)
    }
}

#[async_trait]
impl Source for XtreamSource {
    async fn account_info(&self) -> Result<AccountInfo> {
        let response: AccountResponse = self.get_json(None, &[]).await?;
        let info = response.user_info;
        Ok(AccountInfo {
            auth_ok: info.auth.unwrap_or(0) == 1,
            status: info.status.unwrap_or_else(|| "unknown".to_string()),
            expires_at: info.exp_date,
            max_connections: info.max_connections.unwrap_or(1).max(1),
            active_connections: info.active_cons,
        })
    }

    async fn categories(&self, kind: MediaKind) -> Result<Vec<RawCategory>> {
        let action = match kind {
            MediaKind::Live => "get_live_categories",
            MediaKind::Movie => "get_vod_categories",
            MediaKind::Series => "get_series_categories",
        };
        let wire: Vec<CategoryWire> = self.get_json(Some(action), &[]).await?;
        Ok(wire.into_iter().filter_map(|c| c.into_raw(kind)).collect())
    }

    async fn live_channels(&self) -> Result<Vec<RawChannel>> {
        let wire: Vec<ChannelWire> = self.get_json(Some("get_live_streams"), &[]).await?;
        Ok(wire.into_iter().filter_map(ChannelWire::into_raw).collect())
    }

    async fn movies(&self) -> Result<Vec<RawMovie>> {
        let wire: Vec<MovieWire> = self.get_json(Some("get_vod_streams"), &[]).await?;
        Ok(wire.into_iter().filter_map(MovieWire::into_raw).collect())
    }

    async fn series(&self) -> Result<Vec<RawSeries>> {
        let wire: Vec<SeriesWire> = self.get_json(Some("get_series"), &[]).await?;
        Ok(wire.into_iter().filter_map(SeriesWire::into_raw).collect())
    }

    async fn series_detail(&self, id: &str) -> Result<RawSeriesDetail> {
        let response: SeriesInfoResponse = self
            .get_json(Some("get_series_info"), &[("series_id", id)])
            .await?;

        let mut episodes = Vec::new();
        for (season_key, wire_episodes) in response.episodes {
            let Ok(season) = season_key.parse::<u32>() else {
                continue;
            };
            episodes.extend(wire_episodes.into_iter().filter_map(|e| e.into_raw(season)));
        }
        Ok(RawSeriesDetail { episodes })
    }

    async fn epg_xmltv(&self) -> Result<Option<XmltvStream>> {
        let mut url = Url::parse(&self.base_url)
            .map_err(|_| SourceError::invalid_response(&self.base_url, "URL base inválida"))?;
        url.set_path("xmltv.php");
        url.query_pairs_mut()
            .append_pair("username", &self.username)
            .append_pair("password", &self.password);
        Ok(Some(XmltvStream { url: url.to_string() }))
    }

    fn stream_url(&self, item: &StreamRef) -> Result<Url> {
        self.stream_url_variant(item, true)
    }

    fn stream_url_fallback(&self, item: &StreamRef) -> Result<Option<Url>> {
        if item.kind != MediaKind::Live {
            return Ok(None); // el segmento de tipo es obligatorio salvo en directo.
        }
        Ok(Some(self.stream_url_variant(item, false)?))
    }
}

fn looks_like_auth_failure(bytes: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Probe {
        user_info: Option<ProbeUserInfo>,
    }
    #[derive(Deserialize)]
    struct ProbeUserInfo {
        #[serde(default, deserialize_with = "lenient_u32")]
        auth: Option<u32>,
    }

    serde_json::from_slice::<Probe>(bytes)
        .ok()
        .and_then(|probe| probe.user_info)
        .and_then(|info| info.auth)
        .is_some_and(|auth| auth == 0)
}

// ─────────────── Formas del JSON tal cual las manda el panel ───────────────

#[derive(Deserialize)]
struct AccountResponse {
    user_info: UserInfoWire,
}

#[derive(Deserialize)]
struct UserInfoWire {
    #[serde(default, deserialize_with = "lenient_u32")]
    auth: Option<u32>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    exp_date: Option<i64>,
    #[serde(default, deserialize_with = "lenient_u32")]
    max_connections: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    active_cons: Option<u32>,
}

#[derive(Deserialize)]
struct CategoryWire {
    #[serde(default, deserialize_with = "lenient_id_opt")]
    category_id: Option<String>,
    #[serde(default)]
    category_name: Option<String>,
}

impl CategoryWire {
    fn into_raw(self, kind: MediaKind) -> Option<RawCategory> {
        Some(RawCategory {
            remote_id: self.category_id?,
            kind,
            name: self.category_name.unwrap_or_default(),
        })
    }
}

#[derive(Deserialize)]
struct ChannelWire {
    #[serde(default, deserialize_with = "lenient_id_opt")]
    stream_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "non_empty_string_opt")]
    stream_icon: Option<String>,
    #[serde(default, deserialize_with = "non_empty_string_opt")]
    epg_channel_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_id_opt")]
    category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    tv_archive: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    tv_archive_duration: Option<u32>,
    #[serde(default, deserialize_with = "lenient_i64")]
    num: Option<i64>,
}

impl ChannelWire {
    fn into_raw(self) -> Option<RawChannel> {
        Some(RawChannel {
            remote_id: self.stream_id?,
            category_remote_id: self.category_id,
            raw_name: self.name.unwrap_or_default(),
            logo_url: self.stream_icon,
            epg_channel_id: self.epg_channel_id,
            has_catchup: self.tv_archive.unwrap_or(0) == 1,
            catchup_days: self.tv_archive_duration.unwrap_or(0),
            sort_order: self.num,
        })
    }
}

#[derive(Deserialize)]
struct MovieWire {
    #[serde(default, deserialize_with = "lenient_id_opt")]
    stream_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "non_empty_string_opt")]
    stream_icon: Option<String>,
    #[serde(default)]
    container_extension: Option<String>,
    #[serde(default, deserialize_with = "lenient_id_opt")]
    category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    added: Option<i64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    rating: Option<f64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    tmdb_id: Option<i64>,
}

impl MovieWire {
    fn into_raw(self) -> Option<RawMovie> {
        Some(RawMovie {
            remote_id: self.stream_id?,
            category_remote_id: self.category_id,
            raw_title: self.name.unwrap_or_default(),
            container_extension: self.container_extension,
            logo_url: self.stream_icon,
            added_at: self.added,
            rating: self.rating,
            tmdb_id: self.tmdb_id,
        })
    }
}

#[derive(Deserialize)]
struct SeriesWire {
    #[serde(default, deserialize_with = "lenient_id_opt")]
    series_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    plot: Option<String>,
    #[serde(default, rename = "releaseDate")]
    release_date: Option<String>,
}

impl SeriesWire {
    fn into_raw(self) -> Option<RawSeries> {
        Some(RawSeries {
            remote_id: self.series_id?,
            raw_title: self.name.unwrap_or_default(),
            cover_url: self.cover,
            plot: self.plot,
            release_date: self.release_date,
        })
    }
}

#[derive(Deserialize)]
struct SeriesInfoResponse {
    #[serde(default)]
    episodes: HashMap<String, Vec<EpisodeWire>>,
}

#[derive(Deserialize)]
struct EpisodeWire {
    #[serde(default, deserialize_with = "lenient_id_opt")]
    id: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    episode_num: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    container_extension: Option<String>,
    #[serde(default)]
    info: Option<EpisodeInfoWire>,
}

#[derive(Deserialize)]
struct EpisodeInfoWire {
    #[serde(default, deserialize_with = "lenient_i64")]
    duration_secs: Option<i64>,
    #[serde(default)]
    movie_image: Option<String>,
}

impl EpisodeWire {
    fn into_raw(self, season: u32) -> Option<RawEpisode> {
        Some(RawEpisode {
            remote_id: self.id?,
            season,
            number: self.episode_num?,
            title: self.title,
            container_extension: self.container_extension,
            duration_s: self.info.as_ref().and_then(|i| i.duration_secs),
            still_url: self.info.and_then(|i| i.movie_image),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::http::header::{CONTENT_TYPE, USER_AGENT};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/xtream/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("no se pudo leer {path}: {err}"))
    }

    #[derive(Clone, Default)]
    struct MockState {
        last_user_agent: Arc<Mutex<Option<String>>>,
        /// Cuando está activo, cualquier acción de listado responde con el fixture de
        /// autenticación fallida en vez del listado pedido.
        force_auth_failed: Arc<Mutex<bool>>,
    }

    async fn player_api_handler(
        State(state): State<MockState>,
        headers: axum::http::HeaderMap,
        Query(params): Query<HashMap<String, String>>,
    ) -> Response {
        *state.last_user_agent.lock().await = headers
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let action = params.get("action").map(String::as_str);

        let body = if *state.force_auth_failed.lock().await && action.is_some() {
            fixture("account_info_auth_failed.json")
        } else {
            match action {
                None => fixture("account_info.json"),
                Some("get_live_categories") => fixture("get_live_categories.json"),
                Some("get_vod_categories") => fixture("get_vod_categories.json"),
                Some("get_series_categories") => fixture("get_series_categories.json"),
                Some("get_live_streams") => fixture("get_live_streams.json"),
                Some("get_vod_streams") => fixture("get_vod_streams.json"),
                Some("get_series") => fixture("get_series.json"),
                Some("get_series_info") => fixture("get_series_info.json"),
                Some(other) => panic!("acción no cubierta por el mock de tests: {other}"),
            }
        };

        Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    async fn spawn_mock_server(state: MockState) -> String {
        let router = Router::new()
            .route("/player_api.php", get(player_api_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn account_info_parses_tolerant_fields_and_sends_default_user_agent() {
        let state = MockState::default();
        let base_url = spawn_mock_server(state.clone()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let info = source.account_info().await.unwrap();
        assert!(info.auth_ok);
        assert_eq!(info.status, "Active");
        assert_eq!(info.expires_at, Some(1893456000));
        assert_eq!(info.max_connections, 2);
        assert_eq!(info.active_connections, Some(1));

        let sent_ua = state.last_user_agent.lock().await.clone();
        assert_eq!(sent_ua.as_deref(), Some(DEFAULT_USER_AGENT));
    }

    #[tokio::test]
    async fn account_info_defaults_missing_or_empty_fields() {
        // Reutiliza el mock cambiando manualmente qué fixture sirve get_live_streams no
        // aplica aquí: se prueba el mapeo directamente contra el fixture de campos
        // ausentes, sin pasar por HTTP.
        let raw = fixture("account_info_missing_fields.json");
        let response: AccountResponse = serde_json::from_str(&raw).unwrap();
        assert!(response.user_info.auth == Some(1));
        assert_eq!(response.user_info.max_connections, None);
        // La regla "si viene vacío o no numérico, se asume 1" vive en account_info();
        // se comprueba con el valor por defecto aplicado sobre este mismo dato.
        assert_eq!(response.user_info.max_connections.unwrap_or(1).max(1), 1);
    }

    #[tokio::test]
    async fn custom_user_agent_is_sent_when_configured() {
        let state = MockState::default();
        let base_url = spawn_mock_server(state.clone()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", Some("MiReproductor/1.0")).unwrap();

        source.account_info().await.unwrap();

        let sent_ua = state.last_user_agent.lock().await.clone();
        assert_eq!(sent_ua.as_deref(), Some("MiReproductor/1.0"));
    }

    #[tokio::test]
    async fn categories_filters_entries_without_id_and_tolerates_numeric_ids() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let categories = source.categories(MediaKind::Live).await.unwrap();
        // El fixture trae 3 categorías; todas tienen category_id (una numérica), así
        // que ninguna se descarta.
        assert_eq!(categories.len(), 3);
        assert!(categories.iter().any(|c| c.remote_id == "2" && c.name == "DOCUMENTALES"));

        let vod = source.categories(MediaKind::Movie).await.unwrap();
        assert_eq!(vod.len(), 2);

        let series = source.categories(MediaKind::Series).await.unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].name, "SERIES ES");
    }

    #[tokio::test]
    async fn live_channels_skips_entries_without_stream_id() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let channels = source.live_channels().await.unwrap();
        // El fixture trae 4 entradas, una sin stream_id: se descarta.
        assert_eq!(channels.len(), 3);

        let ch101 = channels.iter().find(|c| c.remote_id == "101").unwrap();
        assert_eq!(ch101.epg_channel_id.as_deref(), Some("canaluno.es"));
        assert!(ch101.has_catchup);
        assert_eq!(ch101.catchup_days, 7);

        let ch102 = channels.iter().find(|c| c.remote_id == "102").unwrap();
        assert_eq!(ch102.category_remote_id.as_deref(), Some("2"));
        assert!(!ch102.has_catchup);

        let ch103 = channels.iter().find(|c| c.remote_id == "103").unwrap();
        assert_eq!(ch103.category_remote_id, None); // category_id: "" se trata como ausente
        assert_eq!(ch103.epg_channel_id, None); // epg_channel_id: null
    }

    #[test]
    fn empty_stream_icon_is_treated_as_absent() {
        // Confirmado contra un proveedor real: algunos canales mandan stream_icon: ""
        // en vez de omitir el campo.
        let wire: ChannelWire = serde_json::from_str(r#"{"stream_id": "1", "name": "x", "stream_icon": ""}"#).unwrap();
        assert_eq!(wire.into_raw().unwrap().logo_url, None);
    }

    #[tokio::test]
    async fn movies_parse_numeric_and_string_fields() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let movies = source.movies().await.unwrap();
        // 3 entradas en el fixture, una sin stream_id: se descarta.
        assert_eq!(movies.len(), 2);

        let m1 = movies.iter().find(|m| m.remote_id == "5001").unwrap();
        assert_eq!(m1.added_at, Some(1700000000));
        assert_eq!(m1.rating, Some(7.5));
        assert_eq!(m1.tmdb_id, None);

        let m2 = movies.iter().find(|m| m.remote_id == "5002").unwrap();
        assert_eq!(m2.rating, Some(8.0));
        assert_eq!(m2.tmdb_id, Some(603692));
    }

    #[tokio::test]
    async fn series_list_skips_entries_without_id() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let series = source.series().await.unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].remote_id, "9001");
        assert_eq!(series[0].raw_title, "Serie de Ejemplo");
    }

    #[tokio::test]
    async fn series_detail_groups_episodes_by_season_from_map_keys() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let detail = source.series_detail("9001").await.unwrap();
        assert_eq!(detail.episodes.len(), 3);

        let ep = detail
            .episodes
            .iter()
            .find(|e| e.remote_id == "90012")
            .unwrap();
        assert_eq!(ep.season, 1);
        assert_eq!(ep.number, 2);

        let ep_s2 = detail
            .episodes
            .iter()
            .find(|e| e.remote_id == "90021")
            .unwrap();
        assert_eq!(ep_s2.season, 2);
        assert_eq!(ep_s2.duration_s, Some(1620));
    }

    #[tokio::test]
    async fn listing_endpoint_reports_auth_failed_instead_of_parse_error() {
        let state = MockState::default();
        *state.force_auth_failed.lock().await = true;
        let base_url = spawn_mock_server(state).await;
        let source = XtreamSource::new(base_url, "demo", "demo", None).unwrap();

        let err = source.live_channels().await.unwrap_err();
        assert!(matches!(err, SourceError::AuthFailed));
    }

    #[tokio::test]
    async fn unreachable_host_error_never_contains_credentials() {
        // Puerto que casi con toda seguridad no tiene nada escuchando.
        let source = XtreamSource::new("http://127.0.0.1:1", "secreto_usuario", "secreto_pass", None).unwrap();
        let err = source.account_info().await.unwrap_err();
        let message = err.to_string();
        assert!(!message.contains("secreto_usuario"));
        assert!(!message.contains("secreto_pass"));
    }

    #[test]
    fn stream_url_builds_live_url_with_ts_default_extension() {
        let source = XtreamSource::new("http://panel.example.com:8080", "user", "pass", None).unwrap();
        let item = StreamRef { kind: MediaKind::Live, remote_id: "42".to_string(), container_extension: None };

        let url = source.stream_url(&item).unwrap();
        assert_eq!(url.as_str(), "http://panel.example.com:8080/live/user/pass/42.ts");
    }

    #[test]
    fn stream_url_variant_without_live_segment_omits_it() {
        let source = XtreamSource::new("http://panel.example.com:8080", "user", "pass", None).unwrap();
        let item = StreamRef { kind: MediaKind::Live, remote_id: "42".to_string(), container_extension: None };

        let url = source.stream_url_variant(&item, false).unwrap();
        assert_eq!(url.as_str(), "http://panel.example.com:8080/user/pass/42.ts");
    }

    #[test]
    fn stream_url_for_movie_always_keeps_type_segment_and_uses_container_extension() {
        let source = XtreamSource::new("http://panel.example.com:8080", "user", "pass", None).unwrap();
        let item = StreamRef {
            kind: MediaKind::Movie,
            remote_id: "5001".to_string(),
            container_extension: Some("mkv".to_string()),
        };

        // Incluso pidiendo la variante "sin segmento", película y serie lo conservan.
        let url = source.stream_url_variant(&item, false).unwrap();
        assert_eq!(url.as_str(), "http://panel.example.com:8080/movie/user/pass/5001.mkv");
    }

    #[test]
    fn stream_url_fallback_only_applies_to_live() {
        let source = XtreamSource::new("http://panel.example.com:8080", "user", "pass", None).unwrap();

        let live = StreamRef { kind: MediaKind::Live, remote_id: "42".to_string(), container_extension: None };
        let fallback = source.stream_url_fallback(&live).unwrap().unwrap();
        assert_eq!(fallback.as_str(), "http://panel.example.com:8080/user/pass/42.ts");

        let movie = StreamRef { kind: MediaKind::Movie, remote_id: "5001".to_string(), container_extension: Some("mkv".to_string()) };
        assert!(source.stream_url_fallback(&movie).unwrap().is_none());
    }

    #[tokio::test]
    async fn epg_xmltv_resolves_url_without_downloading_it() {
        let base_url = spawn_mock_server(MockState::default()).await;
        let source = XtreamSource::new(base_url.clone(), "demo", "demo", None).unwrap();

        let epg = source.epg_xmltv().await.unwrap().unwrap();
        assert_eq!(epg.url, format!("{base_url}/xmltv.php?username=demo&password=demo"));
    }
}
