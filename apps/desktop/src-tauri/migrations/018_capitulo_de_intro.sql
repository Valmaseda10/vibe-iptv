-- "Saltar intro" (F14): el capítulo de intro del fichero, si la sonda encontró uno con ese
-- título. Nunca se adivina por tiempos ni por patrones: sin capítulo, no hay botón.
ALTER TABLE probe_cache ADD COLUMN intro_start_s REAL;
ALTER TABLE probe_cache ADD COLUMN intro_end_s REAL;

-- Lo sondeado antes no pidió los capítulos: se vuelve a sondear al reproducirlo.
DELETE FROM probe_cache;
