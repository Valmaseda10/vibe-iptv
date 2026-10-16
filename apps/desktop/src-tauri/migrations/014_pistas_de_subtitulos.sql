-- Pistas de subtítulos incrustadas que vio la sonda (F12), como JSON, igual que
-- `audio_tracks_json` (migración 010): lista pequeña, de solo lectura, que se calcula la
-- única vez que ya se inspecciona el stream (regla R1).
ALTER TABLE probe_cache ADD COLUMN subtitle_tracks_json TEXT;

-- Lo sondeado antes no sabe qué subtítulos tiene y nunca los ofrecería. Es una caché: se
-- vacía y cada ítem se vuelve a sondear la próxima vez que se reproduzca de verdad.
DELETE FROM probe_cache;
