//! Remux a fMP4 sobre el proxy local (nivel 2 de la cascada). Flags exactos de
//! `docs/ARCHITECTURE.md`: el vídeo nunca se recodifica, el audio solo si el WebView no
//! lo soporta (AC3, E-AC3, DTS — ver `media::probe::audio_needs_transcode`). El AAC que
//! llega dentro de MPEG-TS se copia con `aac_adtstoasc` (ver `build_remux_command`).

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::util::redact;

/// Duración de cada trozo de la grabación del directo (F13). Corto para que pausar y
/// reanudar no tenga que esperar mucho al primer trozo; ffmpeg corta en el fotograma
/// clave siguiente, así que es un mínimo.
pub const TIMESHIFT_SEGMENT_SECONDS: u32 = 2;

/// Todo lo que decide cómo se remuxea, aparte de la URL.
#[derive(Debug, Clone, Copy, Default)]
pub struct RemuxOptions<'a> {
    /// Códec de la pista de audio que se va a usar, según la sonda.
    pub audio_codec: Option<&'a str>,
    /// Audio que el cliente decodifica él mismo ("ac3", "eac3"): se copia.
    pub client_audio: &'a [String],
    pub live: bool,
    /// Búsqueda en VOD: `-ss` de entrada.
    pub start_seconds: Option<f64>,
    /// Índice relativo de la pista de audio elegida.
    pub audio_track: Option<i64>,
    /// Pista de subtítulos de texto y fichero WebVTT donde dejarla (F12).
    pub subtitles: Option<(i64, &'a Path)>,
    /// Grabación del directo para pausar y rebobinar (F13): carpeta donde dejar los trozos
    /// HLS y su lista, con una ventana de `timeshift_window_seconds`.
    pub timeshift: Option<&'a Path>,
    pub timeshift_window_seconds: u32,
}

/// Construye el comando ffmpeg de remux, sin lanzarlo todavía. Separado de
/// [`spawn_remux`] para poder probar los flags exactos sin depender de tener ffmpeg
/// instalado.
pub fn build_remux_command(ffmpeg_path: &Path, url: &str, user_agent: &str, options: &RemuxOptions) -> Command {
    let RemuxOptions { audio_codec, client_audio, live, start_seconds, audio_track, subtitles, timeshift, timeshift_window_seconds } =
        *options;
    let mut command = Command::new(ffmpeg_path);
    command.args(["-hide_banner", "-loglevel", "error"]).args(["-user_agent", user_agent]);
    if subtitles.is_some() || timeshift.is_some() {
        // stdin está cerrado: si un fichero de salida existiera, ffmpeg preguntaría si
        // sobrescribirlo y abortaría el remux entero sin respuesta.
        command.arg("-y");
    }
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

    // Sin pista elegida ni más salidas, se deja que ffmpeg escoja solas la de vídeo y la
    // de audio (su comportamiento de siempre). En cuanto se pide `-map` una vez hay que
    // pedirlo para las dos, o ffmpeg deja de incluir vídeo del todo. Con subtítulos o
    // grabación hay más de una salida y cada una lleva sus `-map`: la principal también,
    // con la primera pista de audio (la que describe `audio_codec`); el `?` deja pasar un
    // vídeo sin audio.
    let audio_map = audio_track.map(|track| format!("0:a:{track}")).unwrap_or_else(|| "0:a:0?".to_string());
    let explicit_maps = audio_track.is_some() || subtitles.is_some() || timeshift.is_some();
    if explicit_maps {
        command.args(["-map", "0:v:0", "-map", &audio_map]);
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
        // Sin subtítulos dentro del fMP4: ffmpeg metería por su cuenta los de texto como
        // `mov_text`, que el `<video>` no enseña y que, al ser una pista dispersa, puede
        // retener fragmentos esperando a intercalarla.
        .arg("-sn")
        // Fragmentos de medio segundo además de en cada fotograma clave: con canales de
        // GOP largo, esperar al siguiente clave retrasaba el arranque varios segundos.
        .args(["-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof+delay_moov", "-frag_duration", "500000"])
        .arg("pipe:1");

    // Segunda salida del mismo proceso (F12): la pista de subtítulos elegida, a WebVTT,
    // volcada frase a frase para que la API la pueda leer mientras se escribe.
    //
    // `-ss 0` de salida: con `-ss` de entrada y vídeo copiado, la lectura empieza en el
    // fotograma clave anterior y la frase que cae entre ese clave y el punto pedido sale
    // con tiempo negativo; el muxer WebVTT desplazaba entonces la pista entera para
    // evitarlo, y los subtítulos se adelantaban hasta un GOP respecto al vídeo (medido:
    // 1–3 s). Descartar lo anterior al punto pedido deja el cero de los dos en el mismo
    // sitio; solo se pierde la frase que ya estaba en pantalla al buscar.
    if let Some((track, path)) = subtitles {
        command
            .args(["-map", &format!("0:s:{track}"), "-c:s", "webvtt", "-ss", "0", "-flush_packets", "1", "-f", "webvtt"])
            .arg(path);
    }

    // Otra salida del mismo proceso (F13): la grabación del directo, en trozos HLS fMP4 en
    // disco con una ventana deslizante. Misma conexión con el proveedor (R1) y mismo reloj
    // que la salida principal (medido: los dos empiezan en cero), así que el reproductor
    // puede pasar de una a otra en el mismo segundo al pausar o rebobinar. hls.js la
    // reproduce por MSE, que no acepta audio MPEG (MP2/MP3) dentro de MP4 (medido en la
    // WebView2): ese audio, solo aquí, se convierte a AAC.
    if let Some(dir) = timeshift {
        command.args(["-map", "0:v:0", "-map", &audio_map, "-c:v", "copy"]);
        let mse_decodes = matches!(audio_codec, Some("aac") | None) || client_decodes;
        if mse_decodes {
            command.args(["-c:a", "copy"]);
            if audio_codec == Some("aac") {
                command.args(["-bsf:a", "aac_adtstoasc"]);
            }
        } else {
            command.args(["-c:a", "aac", "-b:a", "192k"]);
        }
        let list_size = (timeshift_window_seconds / TIMESHIFT_SEGMENT_SECONDS).max(1);
        // Con barras normales: con las invertidas de Windows, el muxer HLS no sabe sacar
        // la carpeta de la lista y deja `init.mp4` en el directorio de trabajo (medido).
        let dir = dir.to_string_lossy().replace('\\', "/");
        let dir = dir.trim_end_matches('/');
        command
            .arg("-sn")
            .args(["-f", "hls", "-hls_time", &TIMESHIFT_SEGMENT_SECONDS.to_string(), "-hls_list_size", &list_size.to_string()])
            .args(["-hls_flags", "delete_segments+independent_segments+temp_file", "-hls_segment_type", "fmp4"])
            .args(["-hls_fmp4_init_filename", "init.mp4", "-hls_segment_filename"])
            .arg(format!("{dir}/seg-%06d.m4s"))
            .arg(format!("{dir}/live.m3u8"));
    }

    command
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
pub fn spawn_remux(ffmpeg_path: &Path, url: &str, user_agent: &str, options: &RemuxOptions) -> std::io::Result<Child> {
    let mut child = build_remux_command(ffmpeg_path, url, user_agent, options).spawn()?;

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
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, ..Default::default() });
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(args[pos + 1], "copy");
    }

    #[test]
    fn aac_is_copied_with_the_adts_to_mp4_filter() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { audio_codec: Some("aac"), live: true, ..Default::default() });
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "copy");
        assert!(args.windows(2).any(|w| w[0] == "-bsf:a" && w[1] == "aac_adtstoasc"));
    }

    #[test]
    fn other_compatible_audio_is_copied_without_the_aac_filter() {
        for codec in [Some("mp3"), Some("opus"), None] {
            let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { audio_codec: codec, live: true, ..Default::default() });
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
            let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { audio_codec: Some(codec), client_audio: &client, live: true, ..Default::default() }));
            let pos = args.iter().position(|a| a == "-c:a").unwrap();
            assert_eq!(args[pos + 1], "copy", "{codec} no debe convertirse si el cliente lo decodifica");
        }
        // DTS sigue convirtiéndose: el cliente no dijo que lo decodifique.
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { audio_codec: Some("dts"), client_audio: &client, live: true, ..Default::default() }));
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "aac");
    }

    #[test]
    fn audio_transcodes_to_aac_at_192k_when_incompatible() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { audio_codec: Some("ac3"), live: true, ..Default::default() });
        let args = args_of(&command);
        let pos = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[pos + 1], "aac");
        let b_pos = args.iter().position(|a| a == "-b:a").unwrap();
        assert_eq!(args[b_pos + 1], "192k");
    }

    #[test]
    fn output_uses_fragmented_mp4_over_stdout() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, ..Default::default() });
        let args = args_of(&command);
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "mp4"));
        assert!(args.iter().any(|a| a == "frag_keyframe+empty_moov+default_base_moof+delay_moov"));
        assert_eq!(args.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn the_initial_burst_is_never_dropped_and_live_analyzes_less() {
        let live = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, ..Default::default() }));
        let vod = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", &RemuxOptions { ..Default::default() }));
        for args in [&live, &vod] {
            assert!(!args.iter().any(|a| a == "nobuffer"), "nobuffer tira la ráfaga inicial del panel");
            assert!(args.windows(2).any(|w| w[0] == "-frag_duration" && w[1] == "500000"));
        }
        assert!(live.windows(2).any(|w| w[0] == "-analyzeduration" && w[1] == "1M"));
        assert!(vod.windows(2).any(|w| w[0] == "-analyzeduration" && w[1] == "2M"));
    }

    #[test]
    fn user_agent_and_input_url_are_forwarded() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "MiAgente/1.0", &RemuxOptions { live: true, ..Default::default() });
        let args = args_of(&command);
        let ua_pos = args.iter().position(|a| a == "-user_agent").unwrap();
        assert_eq!(args[ua_pos + 1], "MiAgente/1.0");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args[i_pos + 1], "http://x/1.ts");
    }

    #[test]
    fn start_seconds_inserts_input_seek_before_input_url() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", &RemuxOptions { start_seconds: Some(125.5), ..Default::default() });
        let args = args_of(&command);
        let ss_pos = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss_pos + 1], "125.5");
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss_pos < i_pos, "-ss debe ir antes de -i para que sea seek de entrada, no de salida");
    }

    #[test]
    fn no_seek_flag_when_start_seconds_is_none() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", &RemuxOptions { ..Default::default() });
        let args = args_of(&command);
        assert!(!args.iter().any(|a| a == "-ss"));
    }

    #[test]
    fn no_explicit_map_without_an_audio_track_choice() {
        // Comportamiento de siempre: sin pista elegida, ffmpeg decide solo qué pistas
        // incluir — pedir `-map` sin necesidad rompería eso (dejaría de incluir vídeo).
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, ..Default::default() });
        let args = args_of(&command);
        assert!(!args.iter().any(|a| a == "-map"));
    }

    #[test]
    fn selecting_an_audio_track_maps_video_and_that_track_explicitly() {
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, audio_track: Some(1), ..Default::default() });
        let args = args_of(&command);
        let map_positions: Vec<usize> = args.iter().enumerate().filter(|(_, a)| *a == "-map").map(|(i, _)| i).collect();
        assert_eq!(map_positions.len(), 2, "hacen falta los dos -map (vídeo y audio) o ffmpeg deja de incluir vídeo");
        assert_eq!(args[map_positions[0] + 1], "0:v:0");
        assert_eq!(args[map_positions[1] + 1], "0:a:1");
    }

    #[test]
    fn the_live_recording_is_a_rolling_hls_output_with_mse_friendly_audio() {
        let dir = Path::new("C:/tmp/ts");
        let options = |codec: Option<&'static str>| RemuxOptions {
            audio_codec: codec,
            live: true,
            timeshift: Some(dir),
            timeshift_window_seconds: 1800,
            ..Default::default()
        };
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &options(Some("mp2"))));
        let pipe = args.iter().position(|a| a == "pipe:1").unwrap();
        let (live, recording) = (&args[..pipe], &args[pipe + 1..]);
        // El directo copia el MP2 (el `<video>` lo reproduce); la grabación va por MSE, que no.
        assert!(live.windows(2).any(|w| w[0] == "-c:a" && w[1] == "copy"));
        assert!(recording.windows(2).any(|w| w[0] == "-c:a" && w[1] == "aac"));
        assert!(live.windows(2).any(|w| w[0] == "-map" && w[1] == "0:a:0?"), "con dos salidas, mapas explícitos");
        assert!(recording.windows(2).any(|w| w[0] == "-f" && w[1] == "hls"));
        assert!(recording.windows(2).any(|w| w[0] == "-hls_list_size" && w[1] == "900"), "30 min en trozos de 2 s");
        assert!(recording.windows(2).any(|w| w[0] == "-hls_flags" && w[1].contains("delete_segments")), "ventana deslizante");
        assert!(recording.windows(2).any(|w| w[0] == "-hls_segment_type" && w[1] == "fmp4"));
        assert!(recording.last().unwrap().ends_with("live.m3u8"));
        assert!(args.iter().any(|a| a == "-y"));

        // AAC se copia en las dos (con su filtro); sin grabación no cambia nada.
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &options(Some("aac"))));
        let pipe = args.iter().position(|a| a == "pipe:1").unwrap();
        assert!(args[pipe + 1..].windows(2).any(|w| w[0] == "-bsf:a" && w[1] == "aac_adtstoasc"));
        assert!(!args[pipe + 1..].iter().any(|a| a == "aac"));
        let plain = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, ..Default::default() }));
        assert!(!plain.iter().any(|a| a == "hls"));
        assert_eq!(plain.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn subtitles_go_to_a_second_webvtt_output_and_never_into_the_mp4() {
        let path = Path::new("C:/tmp/subs.vtt");
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", &RemuxOptions { subtitles: Some((1, path)), ..Default::default() }));
        assert!(args.iter().any(|a| a == "-y"), "sin -y, ffmpeg pregunta por stdin y aborta");
        let pipe = args.iter().position(|a| a == "pipe:1").unwrap();
        let main_output = &args[..pipe];
        let subtitle_output = &args[pipe + 1..];
        assert!(main_output.iter().any(|a| a == "-sn"));
        assert!(main_output.windows(2).any(|w| w[0] == "-map" && w[1] == "0:v:0"));
        assert!(main_output.windows(2).any(|w| w[0] == "-map" && w[1] == "0:a:0?"));
        assert!(subtitle_output.windows(2).any(|w| w[0] == "-map" && w[1] == "0:s:1"));
        assert!(subtitle_output.windows(2).any(|w| w[0] == "-c:s" && w[1] == "webvtt"));
        assert!(subtitle_output.windows(2).any(|w| w[0] == "-f" && w[1] == "webvtt"));
        assert!(subtitle_output.windows(2).any(|w| w[0] == "-ss" && w[1] == "0"), "sin -ss 0 la pista se desplaza tras una búsqueda");
        assert_eq!(subtitle_output.last().map(String::as_str), Some("C:/tmp/subs.vtt"));

        // Con pista de audio elegida, esa es la que va en la salida principal.
        let args = args_of(&build_remux_command(Path::new("ffmpeg"), "http://x/1.mkv", "ua", &RemuxOptions { audio_track: Some(2), subtitles: Some((0, path)), ..Default::default() }));
        let pipe = args.iter().position(|a| a == "pipe:1").unwrap();
        assert!(args[..pipe].windows(2).any(|w| w[0] == "-map" && w[1] == "0:a:2"));
        assert!(!args[..pipe].iter().any(|a| a == "0:a:0?"));
    }

    #[test]
    fn selecting_audio_track_zero_still_maps_explicitly() {
        // Track 0 es "falsy" en muchos lenguajes — confirma que Some(0) no se confunde
        // con None y se sigue mapeando de forma explícita.
        let command = build_remux_command(Path::new("ffmpeg"), "http://x/1.ts", "ua", &RemuxOptions { live: true, audio_track: Some(0), ..Default::default() });
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
        let mut child = spawn_remux(&ffmpeg_path, &url, "vibe-iptv-test", &RemuxOptions { audio_codec: Some("aac"), ..Default::default() }).unwrap();
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
