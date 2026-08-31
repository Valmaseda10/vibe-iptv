-- Duración y códecs reales de una película, sin abrir ningún stream (`get_vod_info` de
-- Xtream, F "premium"). Por fuente, no por película: dos fuentes de la misma película
-- pueden ser ficheros distintos. `vod_info_checked_at` distingue "no comprobado
-- todavía" de "comprobado y el proveedor no dio nada" (mismo criterio que
-- trailer_checked_at, migración 006).
ALTER TABLE movie_sources ADD COLUMN duration_s INTEGER;
ALTER TABLE movie_sources ADD COLUMN video_codec TEXT;
ALTER TABLE movie_sources ADD COLUMN audio_codec TEXT;
ALTER TABLE movie_sources ADD COLUMN vod_info_checked_at INTEGER;
