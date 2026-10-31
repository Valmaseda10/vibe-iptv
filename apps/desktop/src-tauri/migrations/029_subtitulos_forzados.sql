-- Subtítulos forzados (2026-10-31, ver docs/DECISIONS.md): la sonda guarda ahora el nombre
-- de cada pista y si es de forzados. Las sondas con subtítulos se repiten la próxima vez
-- que se reproduzca el título (sin ellas, el selector enseñaba dos «Español» iguales).
DELETE FROM probe_cache WHERE subtitle_tracks_json IS NOT NULL AND subtitle_tracks_json NOT IN ('', '[]');
