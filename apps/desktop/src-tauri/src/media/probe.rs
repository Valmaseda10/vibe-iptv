//! Inspección de códecs con ffprobe y decisión de nivel de la cascada (ver
//! `docs/ARCHITECTURE.md`, sección "Cascada de reproducción"). El resultado se cachea en
//! `probe_cache` (F6 en `db/queries.rs`) para no volver a abrir el stream solo para
//! inspeccionarlo — probarlo una vez, como parte de la única conexión que el usuario ya
//! pidió abrir, no cuenta como el prefetch que prohíbe R1.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("no se pudo ejecutar ffprobe: {0}")]
    Spawn(std::io::Error),
    #[error("ffprobe no respondió a tiempo")]
    Timeout,
    #[error("ffprobe terminó con error")]
    ExitFailure,
    #[error("la salida de ffprobe no se pudo interpretar")]
    InvalidOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResult {
    /// `None` si el stream no tiene pista de vídeo (audio-only) o ffprobe no la reportó.
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Nombre de formato tal cual lo da ffprobe (`format.format_name`), p. ej.
    /// `"mpegts"`, `"mov,mp4,m4a,3gp,3g2,mj2"`, `"hls,applehttp"`, `"matroska,webm"`.
    pub container: String,
    /// Duración total en segundos (`format.duration`), si ffprobe la da. En directo no
    /// significa nada (ffprobe solo ve el trozo que llegó a analizar) y el llamador debe
    /// ignorarla; en VOD es la única fuente fiable de duración para el reproductor
    /// cuando el nivel 2 remuxea a un stream sin longitud declarada.
    pub duration_seconds: Option<f64>,
}

/// Identificador estable de una URL de stream para la caché `probe_cache`, sin poder
/// filtrar nunca un fragmento del original (regla R5): un hash, no `redact()`. Igual
/// razón que `sources::m3u::opaque_id` — aquí además la URL puede ni siquiera ser HTTP
/// en sentido estricto (una URL Xtream con contraseña en la ruta), así que un hash de
/// contenido es la única forma de tener una clave que nunca pueda filtrar nada.
pub fn stream_cache_key(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("probe:{:016x}", hasher.finish())
}

/// Ejecuta `ffprobe` sobre `url` y extrae los códecs y el contenedor. No abre ninguna
/// conexión adicional a la que el proxy de streaming vaya a usar de todas formas para la
/// reproducción real: esta es la única inspección, y se cachea por stream (ver
/// `stream_cache_key`).
pub async fn probe_url(ffprobe_path: &Path, url: &str, user_agent: &str) -> Result<ProbeResult, ProbeError> {
    let mut command = Command::new(ffprobe_path);
    command
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            "-user_agent",
            user_agent,
        ])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Si el timeout de abajo cancela la espera, tokio mata el proceso al soltar el
        // `Child` en vez de dejarlo huérfano intentando conectar indefinidamente.
        .kill_on_drop(true);

    let child = command.spawn().map_err(ProbeError::Spawn)?;

    let output = tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ProbeError::Timeout)?
        .map_err(ProbeError::Spawn)?;

    if !output.status.success() {
        return Err(ProbeError::ExitFailure);
    }

    parse_ffprobe_json(&output.stdout).ok_or(ProbeError::InvalidOutput)
}

fn parse_ffprobe_json(bytes: &[u8]) -> Option<ProbeResult> {
    #[derive(Deserialize)]
    struct FfprobeOutput {
        #[serde(default)]
        streams: Vec<FfprobeStream>,
        #[serde(default)]
        format: Option<FfprobeFormat>,
    }
    #[derive(Deserialize)]
    struct FfprobeStream {
        codec_type: Option<String>,
        codec_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct FfprobeFormat {
        format_name: Option<String>,
        // ffprobe la manda como cadena ("123.456000"), no como número.
        #[serde(default, deserialize_with = "duration_from_string")]
        duration: Option<f64>,
    }

    fn duration_from_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Option<f64>, D::Error> {
        let raw: Option<String> = Option::deserialize(deserializer)?;
        Ok(raw.and_then(|s| s.parse::<f64>().ok()))
    }

    let parsed: FfprobeOutput = serde_json::from_slice(bytes).ok()?;
    let video_codec = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .and_then(|s| s.codec_name.clone());
    let audio_codec = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .and_then(|s| s.codec_name.clone());
    let format = parsed.format?;
    let container = format.format_name?;

    Some(ProbeResult { video_codec, audio_codec, container, duration_seconds: format.duration })
}

/// Decide el nivel de la cascada (1, 2 o 3) a partir de contenedor y códecs, siguiendo
/// `docs/ARCHITECTURE.md`.
///
/// Simplificación deliberada y documentada como deuda: no hay detección de aceleración
/// por hardware todavía, así que H.265 va siempre a nivel 3 aunque el equipo pudiera
/// decodificarlo — es la opción conservadora, ya que el WebView (Chromium en Windows) no
/// soporta HEVC en MSE en la configuración por defecto.
pub fn decide_tier(container: &str, video_codec: Option<&str>, audio_codec: Option<&str>) -> u8 {
    let is_hls = container.split(',').any(|c| matches!(c.trim(), "hls" | "applehttp"));
    if is_hls {
        return 1;
    }

    let Some(video) = video_codec else {
        return 3; // sin pista de vídeo reconocible: no hay nada que copiar ni remuxear con garantías.
    };
    if video != "h264" {
        return 3;
    }

    let is_mp4_like = container.split(',').any(|c| matches!(c.trim(), "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2"));
    let audio_is_direct_compatible = matches!(audio_code_class(audio_codec), AudioClass::DirectCompatible);

    if is_mp4_like && audio_is_direct_compatible {
        return 1;
    }

    2
}

enum AudioClass {
    DirectCompatible,
    Unknown,
}

fn audio_code_class(audio_codec: Option<&str>) -> AudioClass {
    match audio_codec {
        Some("aac") | Some("mp3") => AudioClass::DirectCompatible,
        Some(_) => AudioClass::Unknown,
        None => AudioClass::DirectCompatible, // sin pista de audio: no hay nada que transcodificar.
    }
}

/// El nivel 2 (remux) recodifica el audio solo si el WebView no lo soporta en fMP4: AC3,
/// E-AC3 o DTS (ver `docs/ARCHITECTURE.md`). Cualquier otro códec de audio se copia tal
/// cual, incluidos los que no reconocemos explícitamente — solo estos tres cuestan CPU
/// de más y son los que de verdad rompen la reproducción si no se tocan.
pub fn audio_needs_transcode(audio_codec: Option<&str>) -> bool {
    matches!(audio_codec, Some("ac3") | Some("eac3") | Some("dts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_cache_key_is_stable_and_never_contains_the_url() {
        let url = "http://panel.example.com:8080/live/juan/hunter2/1.ts";
        let key1 = stream_cache_key(url);
        let key2 = stream_cache_key(url);
        assert_eq!(key1, key2);
        assert!(!key1.contains("juan"));
        assert!(!key1.contains("hunter2"));
    }

    #[test]
    fn stream_cache_key_differs_for_different_urls() {
        let a = stream_cache_key("http://panel.example.com/live/u/p/1.ts");
        let b = stream_cache_key("http://panel.example.com/live/u/p/2.ts");
        assert_ne!(a, b);
    }

    #[test]
    fn hls_manifest_is_always_tier_one() {
        assert_eq!(decide_tier("hls,applehttp", Some("h264"), Some("aac")), 1);
        assert_eq!(decide_tier("hls,applehttp", Some("hevc"), None), 1);
    }

    #[test]
    fn progressive_mp4_h264_aac_is_tier_one() {
        assert_eq!(decide_tier("mov,mp4,m4a,3gp,3g2,mj2", Some("h264"), Some("aac")), 1);
        assert_eq!(decide_tier("mov,mp4,m4a,3gp,3g2,mj2", Some("h264"), Some("mp3")), 1);
    }

    #[test]
    fn mp4_h264_with_incompatible_audio_falls_back_to_tier_two() {
        // El vídeo es compatible, pero el audio (AC3) no: nivel 1 exige los dos.
        assert_eq!(decide_tier("mov,mp4,m4a,3gp,3g2,mj2", Some("h264"), Some("ac3")), 2);
    }

    #[test]
    fn mpegts_live_h264_is_tier_two_regardless_of_audio() {
        assert_eq!(decide_tier("mpegts", Some("h264"), Some("aac")), 2);
        assert_eq!(decide_tier("mpegts", Some("h264"), Some("ac3")), 2);
    }

    #[test]
    fn mkv_h264_is_tier_two_never_tier_one() {
        // MKV nunca es nivel 1 aunque los códecs serían compatibles: el contenedor no lo
        // soporta el <video> del WebView, solo fMP4 tras remux.
        assert_eq!(decide_tier("matroska,webm", Some("h264"), Some("aac")), 2);
    }

    #[test]
    fn hevc_falls_back_to_tier_three() {
        assert_eq!(decide_tier("matroska,webm", Some("hevc"), Some("aac")), 3);
        assert_eq!(decide_tier("mpegts", Some("hevc"), Some("aac")), 3);
    }

    #[test]
    fn exotic_video_codecs_fall_back_to_tier_three() {
        assert_eq!(decide_tier("mpegts", Some("mpeg2video"), Some("mp2")), 3);
        assert_eq!(decide_tier("asf", Some("vc1"), Some("wmav2")), 3);
    }

    #[test]
    fn missing_video_stream_falls_back_to_tier_three() {
        assert_eq!(decide_tier("mpegts", None, Some("aac")), 3);
    }

    #[test]
    fn audio_transcode_only_for_ac3_eac3_and_dts() {
        assert!(audio_needs_transcode(Some("ac3")));
        assert!(audio_needs_transcode(Some("eac3")));
        assert!(audio_needs_transcode(Some("dts")));
        assert!(!audio_needs_transcode(Some("aac")));
        assert!(!audio_needs_transcode(Some("mp3")));
        assert!(!audio_needs_transcode(Some("mp2"))); // no lo menciona ARCHITECTURE.md: se copia tal cual.
        assert!(!audio_needs_transcode(None));
    }

    #[test]
    fn parses_real_shaped_ffprobe_output() {
        let json = br#"{
            "streams": [
                {"codec_type": "video", "codec_name": "h264"},
                {"codec_type": "audio", "codec_name": "aac"}
            ],
            "format": {"format_name": "mpegts"}
        }"#;
        let result = parse_ffprobe_json(json).unwrap();
        assert_eq!(result.video_codec.as_deref(), Some("h264"));
        assert_eq!(result.audio_codec.as_deref(), Some("aac"));
        assert_eq!(result.container, "mpegts");
    }

    #[test]
    fn parses_output_with_no_audio_stream() {
        let json = br#"{
            "streams": [{"codec_type": "video", "codec_name": "h264"}],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2"}
        }"#;
        let result = parse_ffprobe_json(json).unwrap();
        assert_eq!(result.video_codec.as_deref(), Some("h264"));
        assert_eq!(result.audio_codec, None);
    }

    #[test]
    fn rejects_garbage_output() {
        assert!(parse_ffprobe_json(b"no es json").is_none());
        assert!(parse_ffprobe_json(b"{}").is_none()); // sin "format": sin format_name, no hay contenedor que decidir.
    }

    /// Prueba real contra el ffprobe instalado en la máquina (no un fixture): confirma
    /// que el parseo de argumentos y de salida funciona contra el binario de verdad, no
    /// solo contra JSON escrito a mano. Si ffprobe no está instalado (p. ej. en CI sin
    /// el sidecar todavía, ver F10), se omite en vez de fallar.
    #[tokio::test]
    async fn probes_a_local_generated_file_with_real_ffprobe() {
        let Ok(ffprobe_path) = which_on_path("ffprobe") else {
            eprintln!("ffprobe no está en el PATH: prueba omitida (se instala como sidecar en F10)");
            return;
        };
        let Ok(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-probe-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = dir.join("sample.mp4");

        let status = Command::new(&ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=1:size=64x64:rate=5", "-f", "lavfi", "-i", "sine=duration=1"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-t", "1"])
            .arg(&sample)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success(), "no se pudo generar el fichero de prueba con ffmpeg");

        let url = sample.to_string_lossy().to_string();
        let result = probe_url(&ffprobe_path, &url, "vibe-iptv-test").await.unwrap();
        assert_eq!(result.video_codec.as_deref(), Some("h264"));
        assert_eq!(result.audio_codec.as_deref(), Some("aac"));
        assert!(result.container.contains("mp4"));
        assert_eq!(decide_tier(&result.container, result.video_codec.as_deref(), result.audio_codec.as_deref()), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn which_on_path(name: &str) -> Result<std::path::PathBuf, ()> {
        let path_var = std::env::var_os("PATH").ok_or(())?;
        let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        std::env::split_paths(&path_var).map(|dir| dir.join(&exe_name)).find(|p| p.is_file()).ok_or(())
    }
}
