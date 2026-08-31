-- Duración real del stream (segundos), tal cual la da ffprobe (`format.duration`). F6
-- la necesita para poder pintar una barra de búsqueda en VOD remuxeado (nivel 2), donde
-- el <video> nunca conoce la duración por sí mismo (stream sin longitud declarada): los
-- metadatos de TMDB (`runtime_min`) no siempre existen, y aunque existan son un dato
-- editorial aproximado, no el de este fichero concreto.
ALTER TABLE probe_cache ADD COLUMN duration_s REAL;
