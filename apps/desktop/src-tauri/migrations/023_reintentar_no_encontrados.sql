-- Emparejamiento con TMDB mejorado (2026-10-22, ver docs/DECISIONS.md): antes solo se miraba
-- el primer resultado de la búsqueda, con el título traducido y con signos de puntuación,
-- y en el catálogo real quedaba sin emparejar 1 de cada 4 series (también algunas muy
-- conocidas). Lo no encontrado se vuelve a buscar con las reglas nuevas.
DELETE FROM tmdb_cache WHERE response = '"NotFound"';
UPDATE movies SET tmdb_state = 'pending' WHERE tmdb_state = 'not_found';
UPDATE series SET tmdb_state = 'pending' WHERE tmdb_state = 'not_found';
