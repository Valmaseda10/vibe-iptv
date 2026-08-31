/// Tipo de medio, igual que la columna `kind` de `categories` en `docs/DATA-MODEL.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Live,
    Movie,
    Series,
}

/// Resultado de `Source::account_info`. `max_connections` es el dato más importante de
/// toda la integración (ver `docs/XTREAM-API.md`): condiciona el StreamGuard.
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub auth_ok: bool,
    /// Estado tal cual lo manda el proveedor (`Active`, `Expired`, `Banned`, ...). Se
    /// guarda sin interpretar: los paneles varían y no hay que inventar valores.
    pub status: String,
    pub expires_at: Option<i64>,
    /// 1 si el proveedor no manda el dato o no es numérico.
    pub max_connections: u32,
    pub active_connections: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RawCategory {
    pub remote_id: String,
    pub kind: MediaKind,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct RawChannel {
    pub remote_id: String,
    pub category_remote_id: Option<String>,
    pub raw_name: String,
    pub logo_url: Option<String>,
    pub epg_channel_id: Option<String>,
    pub has_catchup: bool,
    pub catchup_days: u32,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RawMovie {
    pub remote_id: String,
    pub category_remote_id: Option<String>,
    pub raw_title: String,
    pub container_extension: Option<String>,
    pub logo_url: Option<String>,
    pub added_at: Option<i64>,
    pub rating: Option<f64>,
    /// Si el proveedor lo da, se usa directamente y se salta la búsqueda por título
    /// (ver `docs/XTREAM-API.md`).
    pub tmdb_id: Option<i64>,
}

// `series_sources` en docs/DATA-MODEL.md no tiene columna de categoría (a diferencia de
// `movie_sources`), así que RawSeries tampoco la lleva.
#[derive(Debug, Clone)]
pub struct RawSeries {
    pub remote_id: String,
    pub raw_title: String,
    pub cover_url: Option<String>,
    pub plot: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawEpisode {
    pub remote_id: String,
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub container_extension: Option<String>,
    pub duration_s: Option<i64>,
    pub still_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawSeriesDetail {
    pub episodes: Vec<RawEpisode>,
}

/// Referencia mínima para construir la URL de reproducción de un ítem.
#[derive(Debug, Clone)]
pub struct StreamRef {
    pub kind: MediaKind,
    pub remote_id: String,
    /// `None` en directo (se asume `.ts`); obligatorio en película/episodio.
    pub container_extension: Option<String>,
}

/// Referencia mínima para construir la URL de catch-up/timeshift (F9). `duration_min`
/// es cuánto pedir a partir de `start_epoch`, no la duración total archivada del canal
/// (eso es `channels.catchup_days`, ya en la base de datos).
#[derive(Debug, Clone)]
pub struct CatchupRef {
    pub remote_id: String,
    pub start_epoch: i64,
    pub duration_min: u32,
}

/// Fuente resuelta del EPG en formato XMLTV. Descargarla y parsearla es trabajo de la
/// fase F7 (`catalog::epg`); aquí solo se resuelve dónde está.
#[derive(Debug, Clone)]
pub struct XmltvStream {
    pub url: String,
}
