//! Descarga y parseo en streaming de la guía XMLTV (paso 8 de `docs/ARCHITECTURE.md`,
//! sección "Sincronización"). Nunca carga el fichero entero en memoria: se descarga a
//! disco por trozos y se parsea desde ahí con `quick-xml`, evento a evento.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flate2::read::GzDecoder;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use rusqlite::Connection;
use tokio::io::AsyncWriteExt;

use crate::db::queries;
use crate::sources::Source;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const BATCH_SIZE: usize = 500;
const PAST_CUTOFF_SECONDS: i64 = 24 * 3600;
const FUTURE_CUTOFF_SECONDS: i64 = 7 * 24 * 3600;

#[derive(Debug, thiserror::Error)]
pub enum EpgError {
    #[error("no se pudo descargar el XMLTV: {0}")]
    Download(String),
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("XML mal formado: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("error de base de datos: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("tarea interna interrumpida: {0}")]
    Task(#[from] tokio::task::JoinError),
}

pub struct EpgSyncSummary {
    pub imported: i64,
    pub channels_without_epg: i64,
}

/// Orquesta el paso 8 de `docs/ARCHITECTURE.md`: descarga, parsea (descartando lo
/// anterior a ayer y lo posterior a siete días) y guarda la guía completa, reemplazando
/// la anterior. `None` si la fuente no ofrece XMLTV (M3U sin `x-tvg-url`, por ejemplo).
pub async fn run_epg_sync(
    conn: Connection,
    source: Arc<dyn Source + Send + Sync>,
    user_agent: String,
    tmp_dir: PathBuf,
) -> Result<(Connection, Option<EpgSyncSummary>), EpgError> {
    let Some(xmltv) = source.epg_xmltv().await.map_err(|err| EpgError::Download(err.to_string()))? else {
        return Ok((conn, None));
    };

    let dest = tmp_dir.join(format!("vibe-iptv-xmltv-{}.tmp", std::process::id()));
    download_xmltv(&xmltv.url, &user_agent, &dest).await?;

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let cutoff_past = now - PAST_CUTOFF_SECONDS;
    let cutoff_future = now + FUTURE_CUTOFF_SECONDS;
    let dest_for_parse = dest.clone();

    let (conn, outcome) = tokio::task::spawn_blocking(move || {
        let outcome = import_into_db(&conn, &dest_for_parse, cutoff_past, cutoff_future);
        (conn, outcome)
    })
    .await?;

    let _ = std::fs::remove_file(&dest); // limpieza del temporal: mejor esfuerzo, no fatal si falla.
    Ok((conn, Some(outcome?)))
}

/// La parte síncrona (parseo + SQL): vive separada de `run_epg_sync` para poder
/// ejecutarla entera dentro de un único `spawn_blocking`, igual que el resto de
/// `catalog::sync`.
fn import_into_db(conn: &Connection, path: &Path, cutoff_past: i64, cutoff_future: i64) -> Result<EpgSyncSummary, EpgError> {
    let mut programmes = Vec::new();
    parse_xmltv(path, cutoff_past, cutoff_future, |p| programmes.push(p))?;

    queries::delete_all_epg_programmes(conn)?;
    for chunk in programmes.chunks(BATCH_SIZE) {
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<queries::EpgProgrammeRow> = chunk.iter().map(programme_to_row).collect();
        queries::insert_epg_programmes_batch(&tx, &rows)?;
        tx.commit()?;
    }

    let channels_without_epg = queries::count_channels_without_epg(conn)?;
    Ok(EpgSyncSummary { imported: programmes.len() as i64, channels_without_epg })
}

fn programme_to_row(p: &Programme) -> queries::EpgProgrammeRow<'_> {
    queries::EpgProgrammeRow {
        channel_id: &p.channel_id,
        start_ts: p.start_ts,
        stop_ts: p.stop_ts,
        title: &p.title,
        subtitle: p.subtitle.as_deref(),
        description: p.description.as_deref(),
        category: p.category.as_deref(),
        icon_url: p.icon_url.as_deref(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Programme {
    pub channel_id: String,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub icon_url: Option<String>,
}

/// Descarga `url` a `dest_path` por trozos, sin acumular el cuerpo en memoria. No
/// descomprime aquí: muchos paneles sirven el `.gz` sin declarar `Content-Encoding`, así
/// que la descompresión se decide al abrir el fichero para parsear (ver
/// `open_possibly_gzipped`), sniffeando los bytes mágicos en vez de fiarse de la cabecera.
pub async fn download_xmltv(url: &str, user_agent: &str, dest_path: &Path) -> Result<(), EpgError> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|err| EpgError::Download(err.to_string()))?;

    let mut response = client.get(url).send().await.map_err(|err| EpgError::Download(err.to_string()))?;
    if !response.status().is_success() {
        return Err(EpgError::Download(format!("HTTP {}", response.status())));
    }

    let mut file = tokio::fs::File::create(dest_path).await?;
    while let Some(chunk) = response.chunk().await.map_err(|err| EpgError::Download(err.to_string()))? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

/// Abre `path` para lectura, descomprimiendo sobre la marcha si empieza por los bytes
/// mágicos de gzip (`1f 8b`). Deja el fichero listo para leerse desde el principio en
/// los dos casos.
fn open_possibly_gzipped(path: &Path) -> Result<Box<dyn BufRead>, EpgError> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 2];
    let read = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;

    if read == 2 && magic == [0x1f, 0x8b] {
        Ok(Box::new(BufReader::new(GzDecoder::new(file))))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Parsea el XMLTV en `path` evento a evento, llamando a `on_programme` por cada
/// `<programme>` cuya ventana se solape con `[cutoff_past, cutoff_future]` (epoch,
/// segundos) — descarta el resto ahí mismo, sin construirlo ni guardarlo. El llamador
/// decide qué hacer con cada uno (aquí: acumular lotes de 500 e insertarlos en
/// transacciones, igual que el resto de `catalog::sync`).
pub fn parse_xmltv(path: &Path, cutoff_past: i64, cutoff_future: i64, mut on_programme: impl FnMut(Programme)) -> Result<(), EpgError> {
    let reader = open_possibly_gzipped(path)?;
    let mut xml = Reader::from_reader(reader);
    xml.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut current: Option<PartialProgramme> = None;
    let mut current_field: Option<Field> = None;

    loop {
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"programme" => {
                current = PartialProgramme::from_start_tag(&e);
            }
            Ok(Event::Start(e)) if current.is_some() => {
                current_field = Field::from_name(e.local_name().as_ref());
            }
            Ok(Event::Empty(e)) if e.local_name().as_ref() == b"icon" => {
                if let Some(partial) = current.as_mut() {
                    partial.icon_url = attribute(&e, b"src");
                }
            }
            Ok(Event::Text(t)) => {
                if let (Some(partial), Some(field)) = (current.as_mut(), current_field) {
                    let text = t.unescape()?.trim().to_string();
                    if !text.is_empty() {
                        field.assign(partial, text);
                    }
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"programme" => {
                if let Some(partial) = current.take() {
                    if let Some(programme) = partial.finish() {
                        if programme.stop_ts >= cutoff_past && programme.start_ts <= cutoff_future {
                            on_programme(programme);
                        }
                    }
                }
                current_field = None;
            }
            Ok(Event::End(_)) => current_field = None,
            Ok(Event::Eof) => break,
            Err(err) => return Err(err.into()),
            _ => {}
        }
        buf.clear();
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Title,
    SubTitle,
    Desc,
    Category,
}

impl Field {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"title" => Some(Field::Title),
            b"sub-title" => Some(Field::SubTitle),
            b"desc" => Some(Field::Desc),
            b"category" => Some(Field::Category),
            _ => None,
        }
    }

    fn assign(self, partial: &mut PartialProgramme, text: String) {
        match self {
            // XMLTV puede repetir <title>/<category> en varios idiomas; se conserva el
            // primero, que suele ser el del idioma principal de la guía.
            Field::Title if partial.title.is_none() => partial.title = Some(text),
            Field::SubTitle if partial.subtitle.is_none() => partial.subtitle = Some(text),
            Field::Desc if partial.description.is_none() => partial.description = Some(text),
            Field::Category if partial.category.is_none() => partial.category = Some(text),
            _ => {}
        }
    }
}

struct PartialProgramme {
    channel_id: String,
    start_ts: Option<i64>,
    stop_ts: Option<i64>,
    title: Option<String>,
    subtitle: Option<String>,
    description: Option<String>,
    category: Option<String>,
    icon_url: Option<String>,
}

impl PartialProgramme {
    fn from_start_tag(e: &BytesStart) -> Option<Self> {
        let channel_id = attribute(e, b"channel")?;
        Some(Self {
            channel_id,
            start_ts: attribute(e, b"start").and_then(|s| parse_xmltv_datetime(&s)),
            stop_ts: attribute(e, b"stop").and_then(|s| parse_xmltv_datetime(&s)),
            title: None,
            subtitle: None,
            description: None,
            category: None,
            icon_url: None,
        })
    }

    /// `None` si faltan datos imprescindibles (canal, horas o título): una fila así no
    /// sirve de nada y no vale la pena que reviente el resto del parseo.
    fn finish(self) -> Option<Programme> {
        Some(Programme {
            channel_id: self.channel_id,
            start_ts: self.start_ts?,
            stop_ts: self.stop_ts?,
            title: self.title?,
            subtitle: self.subtitle,
            description: self.description,
            category: self.category,
            icon_url: self.icon_url,
        })
    }
}

fn attribute(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| a.unescape_value().ok()).map(|v| v.into_owned())
}

/// XMLTV usa `YYYYMMDDHHMMSS +ZZZZ` (a veces sin el espacio antes del signo). Se parsea a
/// mano (sin añadir una dependencia de fechas solo para este formato fijo): año, mes,
/// día, hora, minuto, segundo y desplazamiento en minutos, todo aritmética de enteros.
fn parse_xmltv_datetime(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (datetime_part, offset_minutes) = match raw.find(['+', '-']) {
        Some(sign_pos) if sign_pos >= 14 => {
            let (dt, offset) = raw.split_at(sign_pos);
            (dt.trim(), parse_offset_minutes(offset)?)
        }
        _ => (raw, 0),
    };

    if datetime_part.len() < 14 {
        return None;
    }
    let year: i32 = datetime_part[0..4].parse().ok()?;
    let month: u32 = datetime_part[4..6].parse().ok()?;
    let day: u32 = datetime_part[6..8].parse().ok()?;
    let hour: i64 = datetime_part[8..10].parse().ok()?;
    let minute: i64 = datetime_part[10..12].parse().ok()?;
    let second: i64 = datetime_part[12..14].parse().ok()?;

    let days_since_epoch = days_from_civil(year, month, day)?;
    let seconds_local = days_since_epoch * 86_400 + hour * 3600 + minute * 60 + second;
    Some(seconds_local - (offset_minutes as i64) * 60)
}

fn parse_offset_minutes(offset: &str) -> Option<i32> {
    let offset = offset.trim();
    if offset.len() < 3 {
        return None;
    }
    let sign: i32 = match &offset[0..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let digits: String = offset[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let (hours_str, minutes_str) = if digits.len() >= 4 { (&digits[0..2], &digits[2..4]) } else { (digits.as_str(), "0") };
    let hours: i32 = hours_str.parse().ok()?;
    let minutes: i32 = minutes_str.parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

/// Días desde 1970-01-01 hasta la fecha civil dada (algoritmo de Howard Hinnant,
/// dominio público: https://howardhinnant.github.io/date_algorithms.html). Evita
/// depender de una librería de fechas solo para convertir año/mes/día a un entero.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let mp = ((m as i64 + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vibe-iptv-epg-test-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tv>
  <channel id="canaluno.es"><display-name>Canal Uno</display-name></channel>
  <programme channel="canaluno.es" start="20200101120000 +0000" stop="20200101130000 +0000">
    <title lang="es">Telediario</title>
    <sub-title lang="es">Edición de mediodía</sub-title>
    <desc lang="es">Las noticias del día.</desc>
    <category lang="es">Noticias</category>
    <icon src="http://example.com/icon.png"/>
  </programme>
  <programme channel="canaluno.es" start="20200101130000 +0100" stop="20200101140000 +0100">
    <title lang="es">Sin descripcion</title>
  </programme>
  <programme channel="otro.es" start="20200101000000 +0000" stop="20200101010000 +0000">
    <title lang="es">Demasiado viejo</title>
  </programme>
</tv>"#;

    #[test]
    fn parses_full_programme_with_all_fields() {
        let dir = scratch_dir("full");
        let path = write_fixture(&dir, "guide.xml", SAMPLE_XML);

        let mut programmes = Vec::new();
        // Ventana amplia: no se descarta nada salvo el explícitamente "demasiado viejo".
        parse_xmltv(&path, 1_577_750_000, 1_577_930_000, |p| programmes.push(p)).unwrap();

        let telediario = programmes.iter().find(|p| p.title == "Telediario").unwrap();
        assert_eq!(telediario.channel_id, "canaluno.es");
        assert_eq!(telediario.subtitle.as_deref(), Some("Edición de mediodía"));
        assert_eq!(telediario.description.as_deref(), Some("Las noticias del día."));
        assert_eq!(telediario.category.as_deref(), Some("Noticias"));
        assert_eq!(telediario.icon_url.as_deref(), Some("http://example.com/icon.png"));
        // 2020-01-01T12:00:00Z
        assert_eq!(telediario.start_ts, 1_577_880_000);
        assert_eq!(telediario.stop_ts, 1_577_883_600);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_minimal_programme_without_optional_fields() {
        let dir = scratch_dir("minimal");
        let path = write_fixture(&dir, "guide.xml", SAMPLE_XML);

        let mut programmes = Vec::new();
        parse_xmltv(&path, 1_577_750_000, 1_577_930_000, |p| programmes.push(p)).unwrap();

        let minimal = programmes.iter().find(|p| p.title == "Sin descripcion").unwrap();
        assert_eq!(minimal.subtitle, None);
        assert_eq!(minimal.description, None);
        // 13:00 +0100 == 12:00 UTC
        assert_eq!(minimal.start_ts, 1_577_880_000);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discards_programmes_outside_the_cutoff_window() {
        let dir = scratch_dir("cutoff");
        let path = write_fixture(&dir, "guide.xml", SAMPLE_XML);

        let mut programmes = Vec::new();
        // cutoff_past justo después de que termine el programa "demasiado viejo"
        // (acaba a las 01:00 UTC = 1_577_840_400) pero antes de que empiece Telediario
        // (12:00 UTC): el primero se descarta, el segundo no.
        parse_xmltv(&path, 1_577_850_000, 1_577_930_000, |p| programmes.push(p)).unwrap();

        assert!(!programmes.iter().any(|p| p.title == "Demasiado viejo"));
        assert!(programmes.iter().any(|p| p.title == "Telediario"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_gzip_compressed_file_by_sniffing_magic_bytes_not_extension() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let dir = scratch_dir("gzip");
        let path = dir.join("guide.dat"); // sin extensión .gz a propósito
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(SAMPLE_XML.as_bytes()).unwrap();
        encoder.finish().unwrap();

        let mut programmes = Vec::new();
        parse_xmltv(&path, 1_577_750_000, 1_577_930_000, |p| programmes.push(p)).unwrap();

        assert!(programmes.iter().any(|p| p.title == "Telediario"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_programme_missing_required_fields() {
        let dir = scratch_dir("incomplete");
        let xml = r#"<tv><programme channel="x" start="20200101120000 +0000"><title>Sin stop</title></programme></tv>"#;
        let path = write_fixture(&dir, "guide.xml", xml);

        let mut programmes = Vec::new();
        parse_xmltv(&path, 0, 2_000_000_000, |p| programmes.push(p)).unwrap();
        assert!(programmes.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn days_from_civil_matches_known_epoch_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 1, 1), Some(10957));
        assert_eq!(days_from_civil(2020, 1, 1), Some(18262));
    }

    #[test]
    fn parse_xmltv_datetime_handles_offset_without_space() {
        assert_eq!(parse_xmltv_datetime("20200101120000+0000"), Some(1_577_880_000));
        assert_eq!(parse_xmltv_datetime("20200101120000 +0000"), Some(1_577_880_000));
        assert_eq!(parse_xmltv_datetime("20200101130000 +0100"), Some(1_577_880_000));
    }

    #[tokio::test]
    async fn run_epg_sync_downloads_parses_and_reports_channel_coverage() {
        use crate::db::queries::{self, ProviderKind, TitleFields};
        use crate::db::schema::open_connection_at;
        use crate::sources::model::{AccountInfo, MediaKind, RawCategory, RawChannel, RawMovie, RawSeries, RawSeriesDetail, StreamRef, XmltvStream};
        use async_trait::async_trait;
        use axum::routing::get;
        use axum::Router;
        use std::collections::HashMap;

        // Fixture con fechas relativas a "ahora", para que ningún programa quede fuera
        // de la ventana de corte de `run_epg_sync` (ayer .. +7 días) pase lo que pase.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let start = now + 3600;
        let stop = now + 7200;
        let xml = format!(
            r#"<tv><programme channel="canaluno.es" start="{}" stop="{}"><title>Ahora mismo</title></programme></tv>"#,
            epoch_to_xmltv(start),
            epoch_to_xmltv(stop)
        );

        async fn handler(axum::extract::State(xml): axum::extract::State<Arc<String>>) -> String {
            (*xml).clone()
        }
        let router = Router::new().route("/xmltv.php", get(handler)).with_state(Arc::new(xml));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        struct FakeEpgSource {
            url: String,
        }
        #[async_trait]
        impl Source for FakeEpgSource {
            async fn account_info(&self) -> crate::sources::Result<AccountInfo> {
                unimplemented!()
            }
            async fn categories(&self, _: MediaKind) -> crate::sources::Result<Vec<RawCategory>> {
                unimplemented!()
            }
            async fn live_channels(&self) -> crate::sources::Result<Vec<RawChannel>> {
                unimplemented!()
            }
            async fn movies(&self) -> crate::sources::Result<Vec<RawMovie>> {
                unimplemented!()
            }
            async fn series(&self) -> crate::sources::Result<Vec<RawSeries>> {
                unimplemented!()
            }
            async fn series_detail(&self, _: &str) -> crate::sources::Result<RawSeriesDetail> {
                unimplemented!()
            }
            async fn epg_xmltv(&self) -> crate::sources::Result<Option<XmltvStream>> {
                Ok(Some(XmltvStream { url: self.url.clone() }))
            }
            fn stream_url(&self, _: &StreamRef) -> crate::sources::Result<reqwest::Url> {
                unimplemented!()
            }
        }

        let conn = open_connection_at(Path::new(":memory:")).unwrap();
        let provider_id = queries::insert_provider(&conn, ProviderKind::Xtream, "P", "http://x", None, 1, None).unwrap();
        let with_epg = RawChannel {
            remote_id: "1".into(),
            category_remote_id: None,
            raw_name: "Canal".into(),
            logo_url: None,
            epg_channel_id: Some("canaluno.es".into()),
            has_catchup: false,
            catchup_days: 0,
            sort_order: None,
        };
        let without_epg = RawChannel { remote_id: "2".into(), epg_channel_id: Some("otro.es".into()), ..with_epg.clone() };
        let fields = TitleFields { clean_title: "Canal", year: None, match_key: "n/a", quality: None, language: None };
        queries::upsert_channel(&conn, provider_id, &HashMap::new(), &with_epg, &fields).unwrap();
        queries::upsert_channel(&conn, provider_id, &HashMap::new(), &without_epg, &fields).unwrap();

        let source: Arc<dyn Source + Send + Sync> = Arc::new(FakeEpgSource { url: format!("http://{addr}/xmltv.php") });
        let (_, summary) = run_epg_sync(conn, source, "vibe-iptv-test".to_string(), std::env::temp_dir()).await.unwrap();
        let summary = summary.expect("la fuente sí ofrece XMLTV");

        assert_eq!(summary.imported, 1);
        assert_eq!(summary.channels_without_epg, 1);
    }

    /// Convierte un epoch (segundos UTC) al formato XMLTV `YYYYMMDDHHMMSS +0000`, para
    /// construir fixtures relativos a "ahora" sin depender de una librería de fechas
    /// (mismo espíritu que `days_from_civil`: aritmética de enteros, sin atajos).
    fn epoch_to_xmltv(epoch: i64) -> String {
        let days = epoch.div_euclid(86_400);
        let secs_of_day = epoch.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}{m:02}{d:02}{:02}{:02}{:02} +0000", secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60)
    }

    /// Inverso de `days_from_civil` (mismo algoritmo de Howard Hinnant).
    fn civil_from_days(z: i64) -> (i32, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = if m <= 2 { y + 1 } else { y } as i32;
        (y, m, d)
    }

    /// Prueba real: descarga de verdad no se hace contra un servidor externo en un test
    /// unitario (sería frágil y lento), pero sí se ejercita `download_xmltv` contra un
    /// servidor local que sirve el fixture — confirma el flujo completo por HTTP, con
    /// User-Agent incluido, sin mockear la función.
    #[tokio::test]
    async fn download_xmltv_streams_to_disk_without_loading_it_whole_in_memory_first() {
        use axum::routing::get;
        use axum::Router;

        async fn handler() -> &'static str {
            SAMPLE_XML
        }
        let router = Router::new().route("/xmltv.php", get(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let dir = scratch_dir("download");
        let dest = dir.join("downloaded.xml");
        download_xmltv(&format!("http://{addr}/xmltv.php"), "vibe-iptv-test", &dest).await.unwrap();

        let contents = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(contents, SAMPLE_XML);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
