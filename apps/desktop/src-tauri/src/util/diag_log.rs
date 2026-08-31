//! Registro de diagnóstico exportable (F9). Guarda en memoria una copia de las últimas
//! líneas que ya pasaron por el `fmt` layer normal de `tracing` — no vuelve a tocar el
//! texto, así que hereda la garantía de la regla R5 (ninguna URL de proveedor llega a un
//! log sin pasar antes por [`crate::util::redact::redact`]) en vez de intentar adivinar
//! qué redactar sobre una línea ya formateada, que podría corromper texto inocente (p.
//! ej. un `?` suelto en un mensaje) sin ganar nada en seguridad.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

/// Cuántas líneas conserva como máximo. Suficiente para varias sesiones de reproducción
/// sin crecer sin límite (regla R4: nada sin cota en memoria).
const MAX_LINES: usize = 2000;

#[derive(Clone)]
pub struct DiagnosticsLog {
    lines: Arc<Mutex<VecDeque<String>>>,
}

impl DiagnosticsLog {
    pub fn new() -> Self {
        Self { lines: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LINES))) }
    }

    /// Todas las líneas guardadas, más antigua primero, unidas en un único texto listo
    /// para volcar a fichero.
    pub fn snapshot(&self) -> String {
        let lines = self.lines.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    fn push_line(&self, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut lines = self.lines.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if lines.len() >= MAX_LINES {
            lines.pop_front();
        }
        lines.push_back(line.to_string());
    }
}

impl Default for DiagnosticsLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Escritor de `tracing_subscriber::fmt` que parte la salida en líneas y las va
/// guardando en [`DiagnosticsLog`]. `tracing` puede llamar a `write()` varias veces por
/// evento (o agrupar varios en una sola llamada); por eso acumula en un búfer propio
/// hasta encontrar el salto de línea en vez de asumir una línea completa por llamada.
pub struct DiagLogWriter {
    log: DiagnosticsLog,
    partial: String,
}

impl DiagLogWriter {
    fn new(log: DiagnosticsLog) -> Self {
        Self { log, partial: String::new() }
    }
}

impl io::Write for DiagLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.partial.push_str(&String::from_utf8_lossy(buf));
        while let Some(pos) = self.partial.find('\n') {
            let line = self.partial[..pos].to_string();
            self.log.push_line(&line);
            self.partial.drain(..=pos);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DiagnosticsLog {
    type Writer = DiagLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        DiagLogWriter::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn write_splits_into_lines_across_multiple_calls() {
        let log = DiagnosticsLog::new();
        let mut writer = log.make_writer();
        writer.write_all(b"hola ").unwrap();
        writer.write_all(b"mundo\nsegunda").unwrap();
        writer.write_all(b" linea\n").unwrap();

        assert_eq!(log.snapshot(), "hola mundo\nsegunda linea");
    }

    #[test]
    fn keeps_only_the_most_recent_lines_once_full() {
        let log = DiagnosticsLog::new();
        for i in 0..(MAX_LINES + 10) {
            log.push_line(&format!("linea {i}"));
        }
        let snapshot = log.snapshot();
        assert!(!snapshot.contains("linea 0\n") && !snapshot.starts_with("linea 0\n"));
        assert!(snapshot.contains(&format!("linea {}", MAX_LINES + 9)));
        assert_eq!(snapshot.lines().count(), MAX_LINES);
    }

    #[test]
    fn empty_lines_are_not_stored() {
        let log = DiagnosticsLog::new();
        let mut writer = log.make_writer();
        writer.write_all(b"\n\n").unwrap();
        assert_eq!(log.snapshot(), "");
    }
}
