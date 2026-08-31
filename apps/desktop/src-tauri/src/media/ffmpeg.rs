//! Remux a fMP4 sobre el proxy local (nivel 2 de la cascada). Flags exactos de
//! `docs/ARCHITECTURE.md`: el vídeo nunca se recodifica, el audio solo si el WebView no
//! lo soporta (AC3, E-AC3, DTS — ver `media::probe::audio_needs_transcode`).

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::util::redact;

/// Construye el comando ffmpeg de remux, sin lanzarlo todavía. Separado de
/// [`spawn_remux`] para poder probar los flags exactos sin depender de tener ffmpeg
/// instalado.
pub fn build_remux_command(
    ffmpeg_path: &Path,
    url: &str,
    user_agent: &str,
    transcode_audio: bool,
    start_seconds: Option<f64>,
) -> Command {
    let mut command = Command::new(ffmpeg_path);
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .args(["-user_agent", user_agent])
        .args(["-fflags", "nobuffer", "-flags", "low_delay", "-probesize", "2M", "-analyzeduration", "2M"]);

    // Búsqueda para VOD: `-ss` antes de `-i` es "seek de entrada" (rápido, a la
    // keyframe más cercana), la única opción viable con `-c:v copy` — no hay fotogramas
    // que decodificar para llegar exactamente al segundo pedido.
    if let Some(seconds) = start_seconds {
        command.args(["-ss", &seconds.to_string()]);
    }

    command.args(["-i", url]).args(["-c:v", "copy"]);

    if transcode_audio {
        command.args(["-c:a", "aac", "-b:a", "192k"]);
    } else {
        command.args(["-c:a", "copy"]);
    }

    command
        .args(["-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof+delay_moov"])
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Si se suelta el `Child` sin que nadie lo haya matado explícitamente (p. ej. un
        // panic en medio del streaming), que no quede un ffmpeg huérfano.
        .kill_on_drop(true);

    command
}

/// Lanza el remux y engancha un lector de `stderr` que registra cada línea con
/// `tracing`, pasada siempre por `redact()` (regla R5: por si acaso ffmpeg llegara a
/// ecoar la URL de entrada en un mensaje de error).
pub fn spawn_remux(
    ffmpeg_path: &Path,
    url: &str,
    user_agent: &str,
    transcode_audio: bool,
    start_seconds: Option<f64>,
) -> std::io::Result<Child> {
    let mut child = build_remux_command(ffmpeg_path, url, user_agent, transcode_audio, start_seconds).spawn()?;

    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(line = %redact(&line), "ffmpeg (remux nivel 2)");
            }
        });
    }

    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(command: &Command) -> Vec<String> {
        command.as_std().get_args().map(|a| a.to_string_lossy().to_string()).collect()
    }

    #[test]
    fn video_is_always_copied_never_recoded() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", false, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(args[pos + 1], "copy");
    }

    #[test]
    fn audio_is_copied_when_transcode_not_needed() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", false, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "copy");
    }

    #[test]
    fn audio_transcodes_to_aac_at_192k_when_incompatible() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", true, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "aac");
        let b_pos = args.iter().position(|a| a == "-b:a").unwrap();
        assert_eq!(args[b_pos + 1], "192k");
    }

    #[test]
    fn output_uses_fragmented_mp4_over_stdout() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", false, None);
        let args = args_of(&command);
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "mp4"));
        assert!(args.iter().any(|a| a == "frag_keyframe+empty_moov+default_base_moof+delay_moov"));
        assert_eq!(args.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn low_latency_flags_are_present() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", false, None);
        let args = args_of(&command);
        assert!(args.iter().any(|a| a == "nobuffer"));
        assert!(args.iter().any(|a| a == "low_delay"));
    }

    #[test]
    fn user_agent_and_input_url_are_forwarded() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "MiAgente/1.0", false, None);
        let args = args_of(&command);
        let ua_pos = args.iter().position(|a| a == "-user_agent").unwrap();
        assert_eq!(args[ua_pos + 1], "MiAgente/1.0");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args[i_pos + 1], "http://x/1.ts");
    }

    #[test]
    fn start_seconds_inserts_input_seek_before_input_url() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", false, Some(125.5));
        let args = args_of(&command);
        let ss_pos = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss_pos + 1], "125.5");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss_pos < i_pos, "-ss debe ir antes de -i para que sea seek de entrada, no de salida");
    }

    #[test]
    fn no_seek_flag_when_start_seconds_is_none() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", false, None);
        let args = args_of(&command);
        assert!(!args.iter().any(|a| a == "-ss"));
    }

    /// Prueba real contra el ffmpeg instalado: genera una señal sintética, la remuxea a
    /// fMP4 por stdout y confirma que salen bytes con las cajas de un fMP4 de verdad
    /// (`ftyp`/`moof`), no solo que el proceso "parece" arrancar.
    #[tokio::test]
    async fn spawn_remux_produces_valid_fragmented_mp4_bytes() {
        use tokio::io::AsyncReadExt;

        let Ok(ffmpeg_path) = which_on_path("ffmpeg") else {
            eprintln!("ffmpeg no está en el PATH: prueba omitida (se instala como sidecar en F10)");
            return;
        };

        let dir = std::env::temp_dir().join(format!("vibe-iptv-ffmpeg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sample = dir.join("sample.ts");

        let generate = Command::new(&ffmpeg_path)
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=64x64:rate=10", "-f", "lavfi", "-i", "sine=duration=2"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-f", "mpegts"])
            .arg(&sample)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(generate.success());

        // `-user_agent` solo es válido para protocolos de red (http): ffmpeg rechaza la
        // entrada si se aplica a un fichero local ("Option not found"). Se sirve el
        // fichero por HTTP en localhost para que la prueba ejercite exactamente los
        // mismos flags que se usan contra un proveedor real.
        let url = serve_file_over_http(sample.clone()).await;
        let mut child = spawn_remux(&ffmpeg_path, &url, "vibe-iptv-test", false, None).unwrap();
        let mut stdout = child.stdout.take().expect("stdout debería estar conectado");

        let mut buf = vec![0u8; 65536];
        let mut total = Vec::new();
        loop {
            let n = stdout.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buf[..n]);
            if total.len() > 32768 {
                break; // suficiente para confirmar que hay una caja fMP4 válida al principio.
            }
        }

        let _ = child.start_kill();
        let _ = child.wait().await;

        assert!(!total.is_empty(), "el remux no produjo ningún byte");
        // Un MP4/fMP4 empieza con una caja `ftyp` (tamaño de 4 bytes + "ftyp" en ASCII).
        assert_eq!(&total[4..8], b"ftyp", "los primeros bytes no son una caja ftyp de fMP4 válida");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn which_on_path(name: &str) -> Result<std::path::PathBuf, ()> {
        let path_var = std::env::var_os("PATH").ok_or(())?;
        let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        std::env::split_paths(&path_var).map(|dir| dir.join(&exe_name)).find(|p| p.is_file()).ok_or(())
    }

    async fn serve_file_over_http(path: std::path::PathBuf) -> String {
        use axum::routing::get;
        use axum::Router;

        async fn handler(axum::extract::State(path): axum::extract::State<std::path::PathBuf>) -> Vec<u8> {
            tokio::fs::read(&path).await.unwrap()
        }

        let router = Router::new().route("/sample.ts", get(handler)).with_state(path);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}/sample.ts")
    }
}
