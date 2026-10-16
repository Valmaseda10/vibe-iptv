//! Subtítulos incrustados (F12). El remux de nivel 2 saca la pista elegida a un WebVTT
//! temporal como segunda salida del mismo ffmpeg (sin otra conexión con el proveedor,
//! regla R1); esto lo lee mientras se escribe y lo trocea en frases para el reproductor.

use std::path::{Path, PathBuf};

/// Una frase del subtítulo. Tiempos en segundos desde el principio del remux, que es
/// también el cero del `<video>` (con `?start=` los dos empiezan en el punto pedido).
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Carpeta de los WebVTT temporales. Se vacía al arrancar: lo que haya es de sesiones
/// de una ejecución anterior.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("vibe-iptv-subs")
}

/// Fichero nuevo para una sesión de subtítulos. Aleatorio: no lleva nada del ítem ni
/// del proveedor en el nombre (regla R5).
pub fn new_temp_file() -> std::io::Result<PathBuf> {
    use rand::RngExt;
    let dir = temp_dir();
    std::fs::create_dir_all(&dir)?;
    let suffix: u64 = rand::rng().random();
    Ok(dir.join(format!("{}-{suffix:016x}.vtt", std::process::id())))
}

/// Borra los WebVTT que dejó una ejecución anterior.
pub fn clear_temp_dir() {
    let _ = std::fs::remove_dir_all(temp_dir());
}

/// Frases completas de un WebVTT que ffmpeg todavía está escribiendo. El muxer escribe
/// cada frase de una vez (`-flush_packets 1`) terminada en salto de línea; la línea en
/// blanco que la separa de la siguiente llega con la siguiente, así que esperar a ella
/// retrasaría cada frase hasta que hubiera otra. Solo se descarta lo que va detrás del
/// último salto de línea: lo que se esté escribiendo justo ahora.
pub fn parse_complete_cues(vtt: &str) -> Vec<Cue> {
    let Some(last_newline) = vtt.rfind('\n') else { return Vec::new() };
    let normalized = vtt[..=last_newline].replace("\r\n", "\n");
    normalized.split("\n\n").filter_map(parse_block).collect()
}

/// Lee el fichero de una sesión. Vacío si ffmpeg aún no lo ha creado.
pub fn read_cues(path: &Path) -> Vec<Cue> {
    match std::fs::read(path) {
        Ok(bytes) => parse_complete_cues(&String::from_utf8_lossy(&bytes)),
        Err(_) => Vec::new(),
    }
}

fn parse_block(block: &str) -> Option<Cue> {
    let mut lines = block.lines().skip_while(|line| !line.contains("-->"));
    let (start, rest) = lines.next()?.split_once("-->")?;
    // Tras el tiempo final pueden venir ajustes de posición ("align:start ...").
    let end = rest.split_whitespace().next()?;
    let text = lines.collect::<Vec<_>>().join("\n");
    if text.trim().is_empty() {
        return None;
    }
    Some(Cue { start: parse_timestamp(start.trim())?, end: parse_timestamp(end)?, text })
}

/// `hh:mm:ss.mmm` o `mm:ss.mmm`.
fn parse_timestamp(value: &str) -> Option<f64> {
    value.split(':').try_fold(0.0, |total, part| Some(total * 60.0 + part.parse::<f64>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_cues_with_hours_minutes_and_settings() {
        let vtt = "WEBVTT\n\n00:00:01.500 --> 00:00:03.000\nHola\n\n01:02:03.250 --> 01:02:05.000 align:start\n<i>Dos</i>\nlíneas\n\n";
        let cues = parse_complete_cues(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: 1.5, end: 3.0, text: "Hola".into() });
        assert_eq!(cues[1].start, 3723.25);
        assert_eq!(cues[1].end, 3725.0);
        assert_eq!(cues[1].text, "<i>Dos</i>\nlíneas");
    }

    #[test]
    fn the_newest_cue_counts_before_the_blank_line_that_follows_it() {
        // Así lo deja ffmpeg tras escribir la última frase: sin línea en blanco detrás.
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000\nUna\n\n00:03.000 --> 00:04.000\nOtra\n";
        let cues = parse_complete_cues(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1].text, "Otra");
    }

    #[test]
    fn a_cue_still_being_written_is_left_for_the_next_read() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000\nUna\n\n00:03.000 --> 00:04.000\nA me";
        let cues = parse_complete_cues(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start, 1.0);
    }

    #[test]
    fn identifiers_crlf_and_empty_cues_are_handled() {
        let vtt = "WEBVTT\r\n\r\n7\r\n00:00:05.000 --> 00:00:06.000\r\nCon id\r\n\r\n00:00:07.000 --> 00:00:08.000\r\n\r\n";
        let cues = parse_complete_cues(vtt);
        assert_eq!(cues, vec![Cue { start: 5.0, end: 6.0, text: "Con id".into() }]);
    }

    #[test]
    fn missing_file_reads_as_no_cues() {
        assert!(read_cues(Path::new("no-existe-vibe-iptv.vtt")).is_empty());
    }
}
