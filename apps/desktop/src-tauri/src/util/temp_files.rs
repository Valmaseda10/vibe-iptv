//! Restos temporales de ejecuciones anteriores (grabaciones del directo y del rebobinado).

use std::path::Path;

/// Borra de `dir` lo que no sea de este proceso. Los nombres empiezan por el pid de quien
/// los creó (`<pid>-...`): lo de este proceso puede estar en uso, lo demás quedó de un
/// cierre a la fuerza.
pub fn remove_other_processes_entries(dir: &Path) {
    let own = format!("{}-", std::process::id());
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&own) {
            continue;
        }
        let path = entry.path();
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_entries_from_other_processes_are_removed() {
        let dir = std::env::temp_dir().join(format!("vibe-iptv-temp-files-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join(format!("{}-abc.ts", std::process::id()));
        let stale_file = dir.join("1-abc.ts");
        let stale_dir = dir.join("1-def");
        std::fs::write(&mine, b"x").unwrap();
        std::fs::write(&stale_file, b"x").unwrap();
        std::fs::create_dir_all(stale_dir.join("dentro")).unwrap();
        remove_other_processes_entries(&dir);
        assert!(mine.exists());
        assert!(!stale_file.exists() && !stale_dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
