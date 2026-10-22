-- Selector de fuente del directo (2026-10-22, ver docs/DECISIONS.md): las versiones de un
-- canal son las que comparten nombre una vez quitada la calidad («LA 1 HD», «LA 1 4K»).
-- Sin índice, cada cambio de canal recorría la tabla entera (regla R4).
CREATE INDEX IF NOT EXISTS idx_channels_name ON channels(display_name COLLATE NOCASE);
