pub mod error;
pub mod m3u;
pub mod model;
pub mod serde_helpers;
pub mod xtream;

pub use error::{Result, SourceError};
pub use model::{
    AccountInfo, CatchupRef, MediaKind, RawCategory, RawChannel, RawMovie, RawSeries, RawSeriesDetail,
    StreamRef, VodInfo, XmltvStream,
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

    /// URL de catch-up/timeshift (F9). Por defecto, "no soportado" (`NotFound`): M3U no
    /// tiene forma estándar de pedirlo, así que ni lo intenta. `XtreamSource` la
    /// sobreescribe; el llamador siempre comprueba `channels.has_catchup` antes de pedir
    /// esto, así que llegar aquí sin soporte real ya sería un error de todas formas.
    fn catchup_url(&self, item: &CatchupRef) -> Result<Url> {
        Err(SourceError::NotFound { remote_id: item.remote_id.clone() })
    }

    /// Forma alternativa del catch-up, igual que `stream_url_fallback` para el directo:
    /// los paneles Xtream documentan dos formas de URL (ver `docs/XTREAM-API.md`) y cuál
    /// funciona varía por versión de panel.
    fn catchup_url_fallback(&self, _item: &CatchupRef) -> Result<Option<Url>> {
        Ok(None)
    }

    /// Duración y códecs de una película, sin abrir ningún stream (`get_vod_info` en
    /// Xtream). Bajo demanda al abrir la ficha (`server::api`), nunca durante la
    /// sincronización general — regla R4, con catálogos de hasta 100.000 títulos no
    /// tiene sentido una llamada de más por título que puede que nadie abra nunca.
    ///
    /// `docs/XTREAM-API.md` solo documenta que la respuesta trae "duración, códecs", sin
    /// dar la forma exacta del JSON — varía entre versiones de panel y no hay una
    /// muestra real con la que contrastar. Por eso el valor por defecto (M3U, que no
    /// tiene ningún endpoint equivalente) es "sin información" y no un error, y
    /// `XtreamSource` la sobreescribe con un parseo deliberadamente tolerante: un campo
    /// que falte o tenga una forma distinta a la esperada da `None`, nunca rompe la
    /// ficha ni la sincronización.
    async fn vod_info(&self, _remote_id: &str) -> Result<VodInfo> {
        Ok(VodInfo::default())
    }
}
