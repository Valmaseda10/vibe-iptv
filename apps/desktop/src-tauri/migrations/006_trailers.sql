-- Tráiler de YouTube (F "premium" — informe de I+D): se guarda solo la clave del vídeo
-- de YouTube, nunca el vídeo en sí. `trailer_checked_at` distingue "no comprobado
-- todavía" (NULL) de "comprobado y no hay tráiler" (columna de clave a NULL pero
-- checked_at con fecha) — sin esto, cada apertura de la ficha volvería a preguntarle a
-- TMDB aunque ya se supiera que no hay tráiler.
ALTER TABLE movies ADD COLUMN youtube_trailer_key TEXT;
ALTER TABLE movies ADD COLUMN trailer_checked_at INTEGER;
ALTER TABLE series ADD COLUMN youtube_trailer_key TEXT;
ALTER TABLE series ADD COLUMN trailer_checked_at INTEGER;
