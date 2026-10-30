-- «Nuevos episodios» (2026-10-30, ver docs/DECISIONS.md): cuándo cambió cada serie en el
-- proveedor (`last_modified` de `get_series`, sube al añadir episodios), sin pedir la ficha
-- de cada una. `series.updated_at` es el más reciente de sus fuentes.
ALTER TABLE series_sources ADD COLUMN last_modified INTEGER;
ALTER TABLE series ADD COLUMN updated_at INTEGER;
