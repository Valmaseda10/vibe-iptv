-- Tráileres también en inglés (2026-10-30, ver docs/DECISIONS.md): antes solo se pedían en
-- español y muchos títulos se quedaban sin ninguno. Se vuelven a mirar los que no tienen y
-- tienen fondo (los únicos que pueden salir en la portada con el tráiler detrás).
UPDATE movies SET trailer_checked_at = NULL WHERE youtube_trailer_key IS NULL AND tmdb_id IS NOT NULL AND backdrop_path IS NOT NULL;
UPDATE series SET trailer_checked_at = NULL WHERE youtube_trailer_key IS NULL AND tmdb_id IS NOT NULL AND backdrop_path IS NOT NULL;
