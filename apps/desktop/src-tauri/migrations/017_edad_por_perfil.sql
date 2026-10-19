-- Perfil infantil por edad (F14).
--
-- `profiles.max_age`: edad máxima de lo que puede ver el perfil (7, 12, 16...). NULL es
-- "sin límite", y así empiezan todos: `adult_ok = 0` solo significa "sin categorías XXX"
-- (el editor de perfiles lo deja así por defecto), no "es un niño".
ALTER TABLE profiles ADD COLUMN max_age INTEGER;

-- `age_min`: edad mínima del título, sacada de `age_rating` (ver `catalog::age`). NULL es
-- "sin clasificación conocida", y un perfil con límite de edad nunca lo ve: no se puede
-- dar por apto lo que no se sabe. La rellena el paso de extras de TMDB.
ALTER TABLE movies ADD COLUMN age_min INTEGER;
ALTER TABLE series ADD COLUMN age_min INTEGER;
CREATE INDEX idx_movies_age_min ON movies(age_min);
CREATE INDEX idx_series_age_min ON series(age_min);
