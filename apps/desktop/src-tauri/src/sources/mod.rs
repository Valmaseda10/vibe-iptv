pub mod error;
pub mod m3u;
pub mod model;
pub mod serde_helpers;
pub mod xtream;

pub use error::Result;
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
}
