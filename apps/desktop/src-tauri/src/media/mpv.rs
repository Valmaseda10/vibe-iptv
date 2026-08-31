//! Respaldo mpv por IPC (nivel 3 de la cascada, F9 — ver `docs/ARCHITECTURE.md`, sección
//! "Cascada de reproducción"). mpv se lanza en su propia ventana nativa, fuera de la
//! del WebView: empotrarlo es trabajo de una fase posterior (deuda de diseño conocida).
//!
//! **Sin verificar de verdad**: no hay binario de mpv instalado en la máquina donde se
//! ha escrito esto. El comando se construye siguiendo la documentación de mpv (flags
//! `--input-ipc-server`/`--force-window`), y los flags en sí están testeados, pero
//! lanzarlo de verdad y comprobar que aparece una ventana con vídeo e IPC solo lo puede
//! confirmar quien tenga mpv a mano. Ver `docs/DECISIONS.md`, F9.

use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};

/// Construye el comando de mpv, sin lanzarlo todavía — mismo motivo que
/// `media::ffmpeg::build_remux_command`: poder revisar los flags exactos sin depender de
/// tener el binario instalado.
///
/// `url` es la URL real del proveedor, no la de `/stream/*`: mpv existe precisamente
/// para lo que ffmpeg no puede remuxear sin recodificar (H.265 sin aceleración por
/// hardware, VC-1, contenedores raros), así que pasar por el proxy de nivel 2 no tendría
/// sentido. Pasarla como argumento de proceso (nunca registrada, nunca en un log) es
/// coherente con cómo ya se le pasa a ffmpeg en el nivel 2 (regla R5).
pub fn build_launch_command(mpv_path: &Path, url: &str, user_agent: &str, ipc_pipe_path: &str) -> Command {
    let mut command = Command::new(mpv_path);
    command
        .arg(format!("--input-ipc-server={ipc_pipe_path}"))
        .arg(format!("--user-agent={user_agent}"))
        // Ventana propia siempre, aunque el audio tarde en decidirse: sin esto mpv
        // puede arrancar sin ventana visible en algunos formatos.
        .arg("--force-window=yes")
        .arg("--title=VIBE IPTV")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Si se suelta el `Child` sin haberlo matado explícitamente, que no quede un
        // mpv huérfano (mismo motivo que en `media::ffmpeg`).
        .kill_on_drop(true);
    command
}

pub fn spawn_mpv(mpv_path: &Path, url: &str, user_agent: &str, ipc_pipe_path: &str) -> std::io::Result<Child> {
    build_launch_command(mpv_path, url, user_agent, ipc_pipe_path).spawn()
}

/// Ruta de tubería con nombre única para el IPC de esta sesión — no colisiona con otra
/// instancia de la app corriendo a la vez en la misma máquina.
#[cfg(windows)]
pub fn new_ipc_pipe_path() -> String {
    format!(r"\\.\pipe\vibe-iptv-mpv-{}", std::process::id())
}

#[cfg(not(windows))]
pub fn new_ipc_pipe_path() -> String {
    std::env::temp_dir().join(format!("vibe-iptv-mpv-{}.sock", std::process::id())).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(command: &Command) -> Vec<String> {
        command.as_std().get_args().map(|a| a.to_string_lossy().to_string()).collect()
    }

    #[test]
    fn launch_command_includes_ipc_server_and_url() {
        let command = build_launch_command(Path::new("mpv"), "http://x/live/u/p/1.ts", "vibe-iptv", r"\\.\pipe\test");
        let args = args_of(&command);
        assert!(args.iter().any(|a| a == r"--input-ipc-server=\\.\pipe\test"));
        assert!(args.iter().any(|a| a == "--user-agent=vibe-iptv"));
        assert_eq!(args.last().map(String::as_str), Some("http://x/live/u/p/1.ts"));
    }

    #[test]
    fn launch_command_always_forces_a_window() {
        let command = build_launch_command(Path::new("mpv"), "http://x/1.ts", "ua", "pipe");
        let args = args_of(&command);
        assert!(args.iter().any(|a| a == "--force-window=yes"));
    }

    #[test]
    fn ipc_pipe_path_is_unique_per_process() {
        let path = new_ipc_pipe_path();
        assert!(path.contains(&std::process::id().to_string()));
    }
}
