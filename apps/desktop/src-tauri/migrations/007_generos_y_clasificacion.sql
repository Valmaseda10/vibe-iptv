-- Nombres de género (F "premium"): `movies.genres`/`series.genres` solo guardan ids
-- numéricos de TMDB — sin esta tabla no hay forma de enseñar "Acción" en vez de "28" en
-- ningún sitio. `kind` separa película/serie porque TMDB tiene dos catálogos de género
-- parcialmente distintos (p. ej. "Acción y aventura" solo existe en series).
CREATE TABLE genre_names (
  kind          TEXT NOT NULL CHECK (kind IN ('movie', 'tv')),
  tmdb_genre_id INTEGER NOT NULL,
  name          TEXT NOT NULL,
  PRIMARY KEY (kind, tmdb_genre_id)
);

-- Clasificación por edad (F "premium"): certificación de TMDB (ES si la hay, si no la
-- que TMDB dé primero) — nunca del proveedor IPTV, que no la da. `NULL` es "no
-- comprobada todavía" o "TMDB no tiene clasificación para esto", indistinguibles a
-- propósito: en ambos casos se trata como desconocida, nunca como "apta para todos".
ALTER TABLE movies ADD COLUMN age_rating TEXT;
ALTER TABLE series ADD COLUMN age_rating TEXT;
