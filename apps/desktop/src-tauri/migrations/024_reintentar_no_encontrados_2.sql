-- Emparejamiento con TMDB, segunda vuelta (2026-10-30, ver docs/DECISIONS.md): la búsqueda
-- quita restos del proveedor («V O», «(720 )», «-2020», dos puntos sin espacio) y, si con
-- el año no sale, prueba sin él (solo títulos casi idénticos). Lo no encontrado se vuelve a
-- buscar.
DELETE FROM tmdb_cache WHERE response = '"NotFound"';
UPDATE movies SET tmdb_state = 'pending' WHERE tmdb_state = 'not_found';
UPDATE series SET tmdb_state = 'pending' WHERE tmdb_state = 'not_found';
