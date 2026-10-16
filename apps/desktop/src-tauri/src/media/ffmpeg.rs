//! Remux a fMP4 sobre el proxy local (nivel 2 de la cascada). Flags exactos de
//! `docs/ARCHITECTURE.md`: el vídeo nunca se recodifica, el audio solo si el WebView no
//! lo soporta (AC3, E-AC3, DTS — ver `media::probe::audio_needs_transcode`). El AAC que
//! llega dentro de MPEG-TS se copia con `aac_adtstoasc` (ver `build_remux_command`).

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::util::redact;

/// Construye el comando ffmpeg de remux, sin lanzarlo todavía. Separado de
/// [`spawn_remux`] para poder probar los flags exactos sin depender de tener ffmpeg
/// instalado.
#[allow(clippy::too_many_arguments)]
pub fn build_remux_command(
    ffmpeg_path: &Path,
    url: &str,
    user_agent: &str,
    audio_codec: Option<&str>,
    client_audio: &[String],
    live: bool,
    start_seconds: Option<f64>,
    audio_track: Option<i64>,
) -> Command {
    let mut command = Command::new(ffmpeg_path);
    command.args(["-hide_banner", "-loglevel", "error"]).args(["-user_agent", user_agent]);
    // Sin `-fflags nobuffer`: descarta lo leído mientras se analiza la entrada, y eso es
    // justo la ráfaga de búfer que mandan los paneles al conectar. Medido contra el
    // proveedor de pruebas: primer fragmento a los 4,3 s con él y a los 0,1 s sin él; en
    // VOD además se comía los primeros segundos de cada película y tras cada búsqueda.
    // En directo el análisis se acorta a 1 s: es lo que tarda el primer fotograma si el
    // panel no manda ráfaga.
    if live {
        command.args(["-probesize", "1M", "-analyzeduration", "1M"]);
    } else {
        command.args(["-probesize", "2M", "-analyzeduration", "2M"]);
    }

    // Búsqueda para VOD: `-ss` antes de `-i` es "seek de entrada" (rápido, a la
    // keyframe más cercana), la única opción viable con `-c:v copy` — no hay fotogramas
    // que decodificar para llegar exactamente al segundo pedido.
    if let Some(seconds) = start_seconds {
        command.args(["-ss", &seconds.to_string()]);
    }

    command.args(["-i", url]);

    // Sin pista elegida, se deja que ffmpeg escoja solas la de vídeo y la de audio (su
    // comportamiento de siempre, antes de que existiera el selector — no tocar nada
    // aquí es justo lo que mantiene ese caso común idéntico). En cuanto se pide `-map`
    // una vez hay que pedirlo para las dos, o ffmpeg deja de incluir vídeo del todo.
    if let Some(track) = audio_track {
        command.args(["-map", "0:v:0", "-map", &format!("0:a:{track}")]);
    }

    command.args(["-c:v", "copy"]);

    // El cliente dice qué audio decodifica él mismo (`client_audio`, p. ej. "ac3"): la
    // WebView2 de Windows reproduce AC-3/E-AC-3, y convertirlo a AAC estéreo perdía el
    // 5.1 de las películas y gastaba CPU sin necesidad.
    let client_decodes = audio_codec.is_some_and(|codec| client_audio.iter().any(|c| c == codec));
    if crate::media::probe::audio_needs_transcode(audio_codec) && !client_decodes {
        command.args(["-c:a", "aac", "-b:a", "192k"]);
    } else {
        command.args(["-c:a", "copy"]);
        // El AAC de un MPEG-TS (el directo de casi cualquier proveedor) viaja con
        // cabeceras ADTS, que MP4 no admite. ffmpeg antiguo añadía este filtro solo; el
        // 8.x aborta con "Malformed AAC bitstream" tras escribir solo la cabecera, y el
        // `<video>` recibía un vídeo de duración cero que daba por terminado al instante.
        // Solo con AAC: aplicado a otro códec, ffmpeg falla.
        if audio_codec == Some("aac") {
            command.args(["-bsf:a", "aac_adtstoasc"]);
        }
    }

    command
        // Fragmentos de medio segundo además de en cada fotograma clave: con canales de
        // GOP largo, esperar al siguiente clave retrasaba el arranque varios segundos.
        .args(["-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof+delay_moov", "-frag_duration", "500000"])
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
#[allow(clippy::too_many_arguments)]
pub fn spawn_remux(
    ffmpeg_path: &Path,
    url: &str,
    user_agent: &str,
    audio_codec: Option<&str>,
    client_audio: &[String],
    live: bool,
    start_seconds: Option<f64>,
    audio_track: Option<i64>,
) -> std::io::Result<Child> {
    let mut child = build_remux_command(ffmpeg_path, url, user_agent, audio_codec, client_audio, live, start_seconds, audio_track).spawn()?;

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
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(args[pos + 1], "copy");
    }

    #[test]
    fn aac_is_copied_with_the_adts_to_mp4_filter() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", Some("aac"), &[], true, None, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "copy");
        assert!(args.windows(2).any(|w| w[0] == "-bsf:a" && w[1] == "aac_adtstoasc"));
    }

    #[test]
    fn other_compatible_audio_is_copied_without_the_aac_filter() {
        for codec in [Some("mp3"), Some("opus"), None] {
            let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", codec, &[], true, None, None);
            let args = args_of(&command);
            let pos = args.iter().position(|a| a == "-c:a").unwrap();
            assert_eq!(args[pos + 1], "copy");
            assert!(!args.iter().any(|a| a == "aac_adtstoasc"), "{codec:?}");
        }
    }

    #[test]
    fn dolby_audio_is_copied_when_the_client_decodes_it() {
        let client = ["ac3".to_string(), "eac3".to_string()];
        for codec in ["ac3", "eac3"] {
            let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", Some(codec), &client, true, None, None));
            let pos = args.iter().position(|a| a == "-c:a").unwrap();
            assert_eq!(args[pos + 1], "copy", "{codec} no debe convertirse si el cliente lo decodifica");
        }
        // DTS sigue convirtiéndose: el cliente no dijo que lo decodifique.
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", Some("dts"), &client, true, None, None));
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "aac");
    }

    #[test]
    fn audio_transcodes_to_aac_at_192k_when_incompatible() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", Some("ac3"), &[], true, None, None);
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "aac");
        let b_pos = args.iter().position(|a| a == "-b:a").unwrap();
        assert_eq!(args[b_pos + 1], "192k");
    }

    #[test]
    fn output_uses_fragmented_mp4_over_stdout() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, None);
        let args = args_of(&command);
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "mp4"));
        assert!(args.iter().any(|a| a == "frag_keyframe+empty_moov+default_base_moof+delay_moov"));
        assert_eq!(args.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn the_initial_burst_is_never_dropped_and_live_analyzes_less() {
        let live = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, None));
        let vod = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", None, &[], false, None, None));
        for args in [&live, &vod] {
            assert!(!args.iter().any(|a| a == "nobuffer"), "nobuffer tira la ráfaga inicial del panel");
            assert!(args.windows(2).any(|w| w[0] == "-frag_duration" && w[1] == "500000"));
        }
        assert!(live.windows(2).any(|w| w[0] == "-analyzeduration" && w[1] == "1M"));
        assert!(vod.windows(2).any(|w| w[0] == "-analyzeduration" && w[1] == "2M"));
    }

    #[test]
    fn user_agent_and_input_url_are_forwarded() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "MiAgente/1.0", None, &[], true, None, None);
        let args = args_of(&command);
        let ua_pos = args.iter().position(|a| a == "-user_agent").unwrap();
        assert_eq!(args[ua_pos + 1], "MiAgente/1.0");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args[i_pos + 1], "http://x/1.ts");
    }

    #[test]
    fn start_seconds_inserts_input_seek_before_input_url() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", None, &[], false, Some(125.5), None);
        let args = args_of(&command);
        let ss_pos = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss_pos + 1], "125.5");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss_pos < i_pos, "-ss debe ir antes de -i para que sea seek de entrada, no de salida");
    }

    #[test]
    fn no_seek_flag_when_start_seconds_is_none() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", None, &[], false, None, None);
        let args = args_of(&command);
        assert!(!args.iter().any(|a| a == "-ss"));
    }

    #[test]
    fn no_explicit_map_without_an_audio_track_choice() {
        // Comportamiento de siempre: sin pista elegida, ffmpeg decide solo qué pistas
        // incluir — pedir `-map` sin necesidad rompería eso (dejaría de incluir vídeo).
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, None);
        let args = args_of(&command);
        assert!(!args.iter().any(|a| a == "-map"));
    }

    #[test]
    fn selecting_an_audio_track_maps_video_and_that_track_explicitly() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, Some(1));
        let args = args_of(&command);
        let map_positions: Vec<usize> = args.iter().enumerate().filter(|(_, a)| *a == "-map").map(|(i, _)| i).collect();
        assert_eq!(map_positions.len(), 2, "hacen falta los dos -map (vídeo y audio) o ffmpeg deja de incluir vídeo");
        assert_eq!(args[map_positions[0] + 1], "0:v:0");
        assert_eq!(args[map_positions[1] + 1], "0:a:1");
    }

    #[test]
    fn selecting_audio_track_zero_still_maps_explicitly() {
        // Track 0 es "falsy" en muchos lenguajes — confirma que Some(0) no se confunde
        // con None y se sigue mapeando de forma explícita.
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", None, &[], true, None, Some(0));
        let args = args_of(&command);
        assert!(args.iter().any(|a| a == "-map"));
        let last_map = args.iter().enumerate().filter(|(_, a)| *a == "-map").last().unwrap().0;
        assert_eq!(args[last_map + 1], "0:a:0");
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
        let mut child = spawn_remux(&ffmpeg_path, &url, "vibe-iptv-test", Some("aac"), &[], false, None, None).unwrap();
        let mut stdout = child.stdout.take().expect("stdout debería estar conectado");

        let mut buf = vec![0u8; 65536];
        let mut total = Vec::new();
        loop {
            let n = stdout.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buf[..n]);
            if total.len() > 1_000_000 {
                break;
            }
        }

        let _ = child.start_kill();
        let _ = child.wait().await;

        assert!(!total.is_empty(), "el remux no produjo ningún byte");
        // Un MP4/fMP4 empieza con una caja `ftyp` (tamaño de 4 bytes + "ftyp" en ASCII).
        assert_eq!(&total[4..8], b"ftyp", "los primeros bytes no son una caja ftyp de fMP4 válida");
        // La cabecera sola no prueba nada (ver `media::count_packets`): tiene que haber
        // audio y vídeo de verdad dentro.
        if let Ok(ffprobe_path) = which_on_path("ffprobe") {
            let remuxed = dir.join("remuxed.mp4");
            std::fs::write(&remuxed, &total).unwrap();
            assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "v") > 0, "el remux no lleva vídeo");
            assert!(crate::media::count_packets(&ffprobe_path, &remuxed, "a") > 0, "el remux no lleva audio");
        }

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
