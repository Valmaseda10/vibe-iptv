-- F17, parte 2 (2026-11-01, ver docs/DECISIONS.md): sagas y fichas de actores.
--
-- Sale de TMDB, una consulta por título y en segundo plano (`catalog::people`):
-- el reparto principal, la dirección (o quien creó la serie) y la saga a la que pertenece
-- cada película. No viaja a la nube: cada aparato lo pide a TMDB (R2).

ALTER TABLE movies ADD COLUMN collection_id INTEGER;
ALTER TABLE movies ADD COLUMN credits_checked_at INTEGER;
ALTER TABLE series ADD COLUMN credits_checked_at INTEGER;
CREATE INDEX idx_movies_collection ON movies(collection_id) WHERE collection_id IS NOT NULL;

-- Una saga de TMDB. `parts_json`: todas sus películas en orden de estreno (también las que
-- este catálogo no tiene), pedidas al abrir la saga y guardadas un mes.
CREATE TABLE collections (
  id               INTEGER PRIMARY KEY,   -- id de TMDB
  name             TEXT NOT NULL,
  overview         TEXT,
  poster_path      TEXT,
  backdrop_path    TEXT,
  parts_json       TEXT,
  parts_fetched_at INTEGER
);

CREATE TABLE people (
  id           INTEGER PRIMARY KEY,       -- id de TMDB
  name         TEXT NOT NULL,
  profile_path TEXT,
  department   TEXT
);
CREATE INDEX idx_people_name ON people(name COLLATE NOCASE);

-- Quién sale en cada título: el reparto principal en orden, y la dirección o la creación.
CREATE TABLE credits (
  item_kind  TEXT NOT NULL CHECK (item_kind IN ('movie','series')),
  item_id    INTEGER NOT NULL,
  person_id  INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
  role       TEXT NOT NULL CHECK (role IN ('cast','director','creator')),
  character  TEXT,
  ord        INTEGER NOT NULL,
  PRIMARY KEY (item_kind, item_id, person_id, role)
);
CREATE INDEX idx_credits_person ON credits(person_id);

-- Si un título cambia de ficha de TMDB (emparejamiento corregido), su reparto y su saga
-- eran de la otra: se borran y se vuelven a pedir. Y se van con el título.
CREATE TRIGGER movies_tmdb_changed AFTER UPDATE OF tmdb_id ON movies WHEN OLD.tmdb_id IS NOT NEW.tmdb_id
BEGIN
  DELETE FROM credits WHERE item_kind = 'movie' AND item_id = NEW.id;
  UPDATE movies SET credits_checked_at = NULL, collection_id = NULL WHERE id = NEW.id;
END;
CREATE TRIGGER series_tmdb_changed AFTER UPDATE OF tmdb_id ON series WHEN OLD.tmdb_id IS NOT NEW.tmdb_id
BEGIN
  DELETE FROM credits WHERE item_kind = 'series' AND item_id = NEW.id;
  UPDATE series SET credits_checked_at = NULL WHERE id = NEW.id;
END;
CREATE TRIGGER movies_credits_gone AFTER DELETE ON movies
BEGIN
  DELETE FROM credits WHERE item_kind = 'movie' AND item_id = OLD.id;
END;
CREATE TRIGGER series_credits_gone AFTER DELETE ON series
BEGIN
  DELETE FROM credits WHERE item_kind = 'series' AND item_id = OLD.id;
END;
