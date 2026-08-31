pub mod error;
pub mod m3u;
pub mod model;
pub mod serde_helpers;
pub mod xtream;

pub use error::{Result, SourceError};
pub use model::{
    AccountInfo, MediaKind, RawCategory, RawChannel, RawMovie, RawSeries, RawSeriesDetail,
    StreamRef, XmltvStream,
};

use async_trait::async_trait;
use reqwest::Url;

/// Aísla las diferencias entre proveedores. Ver `docs/ARCHITECTURE.md`, sección "Capa
/// de fuentes". Implementaciones: [`xtream::XtreamSource`] y [`m3u::M3uSource`].
#[async_trait]
pub trait Source {
    async fn account_info(&self) -> Result<AccountInfo>;
    async fn categories(&self, kind: MediaKind) -> Result<Vec<RawCategory>>;
    async fn live_channels(&self) -> Result<Vec<RawChannel>>;
    async fn movies(&self) -> Result<Vec<RawMovie>>;
    async fn series(&self) -> Result<Vec<RawSeries>>;
    async fn series_detail(&self, id: &str) -> Result<RawSeriesDetail>;
    async fn epg_xmltv(&self) -> Result<Option<XmltvStream>>;
    fn stream_url(&self, item: &StreamRef) -> Result<Url>;

    /// Forma alternativa de la URL de stream, si la fuente conoce una. Hoy solo la usa
    /// `XtreamSource` para el directo: algunos paneles antiguos sirven el directo sin el
    /// segmento `/live/` (ver `docs/XTREAM-API.md`), y probar cuál funciona es trabajo
    /// del proxy de stream (F6), no de la capa de fuentes — aquí solo se ofrece la
    /// alternativa, sin abrir ninguna conexión (regla R1).
    fn stream_url_fallback(&self, _item: &StreamRef) -> Result<Option<Url>> {
        Ok(None)
    }
}
