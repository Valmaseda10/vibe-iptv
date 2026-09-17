-- "Títulos similares" en la ficha (F "premium"): guarda los ids de TMDB que sugirió
-- `TmdbClient::movie_extras`/`tv_extras` (misma llamada que trailer+clasificación, ver
-- migración 006 — no una petición aparte), como texto separado por comas en vez de una
-- tabla propia: es una lista pequeña y de solo lectura, y ya hay un ciclo de vida
-- (`trailer_checked_at`) que decide cuándo refrescarla. El cruce contra el catálogo
-- local (qué de esos ids existe de verdad) se recalcula en cada visita a la ficha, no
-- aquí: así un título añadido más tarde puede aparecer sin pedir nada nuevo a TMDB.
ALTER TABLE movies ADD COLUMN similar_tmdb_ids TEXT;
ALTER TABLE series ADD COLUMN similar_tmdb_ids TEXT;
