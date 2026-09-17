-- Selector de pista de audio (F "premium" pendiente desde el plan original de F6: "pistas
-- de audio, subtítulos" nunca se llegó a construir). Se guardan todas las pistas que vio
-- ffprobe la única vez que se inspecciona el stream (regla R1 — nunca una sonda aparte),
-- como JSON: es una lista pequeña y de solo lectura, mismo criterio que
-- `movies.similar_tmdb_ids` en la migración 009, no hace falta una tabla propia.
ALTER TABLE probe_cache ADD COLUMN audio_tracks_json TEXT;
