pub mod ffmpeg;
pub mod mpv;
pub mod probe;
pub mod subtitles;

use std::path::{Path, PathBuf};

/// Localiza un binario auxiliar (`ffmpeg`, `ffprobe`, `mpv`). Primero junto al propio
/// ejecutable: así es como Tauri deja los sidecars declarados en `bundle.externalBin`
/// en el paquete instalado — el sufijo de triple de compilación del fichero fuente en
/// `binaries/` (p. ej. `mpv-x86_64-pc-windows-msvc.exe`) se retira al empaquetar, y el
/// binario queda como `mpv.exe` en la misma carpeta que `vibe-iptv.exe` (misma
/// convención que usa `tauri-plugin-shell` internamente — no hace falta esa
/// dependencia solo para esto, que además obligaría a rehacer `StreamGuard` sobre su
/// modelo de proceso, sin `try_wait`). Si no está ahí (dev, o el usuario lo instaló él
/// mismo), cae al `PATH` del sistema.
pub fn resolve_binary(name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };

    if let Ok(current_exe) = std::env::current_exe() {
        // Bajo `cargo test`, el ejecutable vive en `target/debug/deps/`: sube un nivel
        // para mirar en `target/debug/`, donde sí tendría sentido dejar un sidecar de
        // prueba (mismo ajuste que hace `tauri-plugin-shell` para su propio caso de test).
        let mut dir = current_exe.parent().map(Path::to_path_buf);
        if dir.as_deref().and_then(Path::file_name) == Some(std::ffi::OsStr::new("deps")) {
            dir = dir.and_then(|d| d.parent().map(Path::to_path_buf));
        }
        if let Some(dir) = dir {
            let candidate = dir.join(&exe_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // En desarrollo (`tauri dev`) los sidecars siguen en `src-tauri/binaries/` con el
    // sufijo de la plataforma (`ffmpeg-x86_64-pc-windows-msvc.exe`): solo el instalador
    // los copia junto al ejecutable. Sin esto, desarrollo usaba el ffmpeg que hubiera en el
    // PATH (otra versión que la empaquetada) y avisaba de que faltaba mpv.
    #[cfg(debug_assertions)]
    {
        let prefix = format!("{name}-");
        let dev_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
        if let Ok(entries) = std::fs::read_dir(&dev_dir) {
            let found = entries.flatten().map(|entry| entry.path()).find(|path| {
                let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                file_name.starts_with(&prefix) && (!cfg!(windows) || file_name.ends_with(".exe"))
            });
            if found.is_some() {
                return found;
            }
        }
    }

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

    /// Simula el paquete instalado: un fichero junto al ejecutable (con el ajuste de
    /// `deps/` bajo `cargo test`) debe encontrarse sin tocar el `PATH` para nada.
    #[test]
    fn resolve_binary_finds_a_file_next_to_the_current_executable() {
        let name = "un-sidecar-de-prueba-vibe-iptv";
        let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };

        let mut dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        if dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
            dir = dir.parent().unwrap().to_path_buf();
        }
        let path = dir.join(&exe_name);
        std::fs::write(&path, b"no es un binario real, solo para la prueba").unwrap();

        let found = resolve_binary(name);
        let _ = std::fs::remove_file(&path);

        assert_eq!(found, Some(path));
    }
}

/// Paquetes que `ffprobe` cuenta de verdad en un fichero, para un selector de stream
/// (`"v"`, `"a"`). Para los tests de remux: que la salida empiece por `ftyp`, o incluso
/// que tenga algún fragmento, no prueba nada — sin `aac_adtstoasc`, ffmpeg 8 escribía la
/// cabecera y un fragmento de vídeo y abortaba en el primer paquete de audio.
#[cfg(test)]
pub(crate) fn count_packets(ffprobe: &std::path::Path, file: &std::path::Path, selector: &str) -> usize {
    let output = std::process::Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", selector, "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .expect("no se pudo ejecutar ffprobe");
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| line.trim().trim_end_matches(',').parse::<usize>().ok()).sum()
}
