pub mod ffmpeg;
pub mod mpv;
pub mod probe;

use std::path::PathBuf;

/// Localiza un binario auxiliar (`ffmpeg`, `ffprobe`, `mpv`). En v1, mientras F10 no
/// empaquete los sidecars de Tauri en `src-tauri/binaries/`, se busca en el `PATH` del
/// sistema: es una simplificación de desarrollo, documentada como deuda en
/// `docs/DECISIONS.md`. CLAUDE.md prohíbe asumir el PATH en el producto empaquetado, no
/// en esta fase de desarrollo — F10 sustituye esta función por la ruta del sidecar.
pub fn resolve_binary(name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).map(|dir| dir.join(&exe_name)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_binary_returns_none_for_something_that_does_not_exist() {
        assert!(resolve_binary("un-binario-que-no-existe-vibe-iptv").is_none());
    }
}
