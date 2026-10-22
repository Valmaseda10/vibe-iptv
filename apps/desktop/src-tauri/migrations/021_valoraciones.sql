-- Valoraciones por perfil, como en Netflix (2026-10-22, ver docs/DECISIONS.md):
-- -1 «No es para mí», 1 «Me gusta», 2 «Me encanta». Alimentan las recomendaciones
-- (`catalog::recommendations`) y viajan entre dispositivos por la nube, como favoritos.
-- Sin canales: no tienen clave estable entre proveedores (ver docs/CLOUD-SYNC.md).
CREATE TABLE ratings (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  item_kind   TEXT NOT NULL CHECK (item_kind IN ('movie', 'series')),
  item_id     INTEGER NOT NULL,
  rating      INTEGER NOT NULL CHECK (rating IN (-1, 1, 2)),
  rated_at    INTEGER NOT NULL,
  PRIMARY KEY (profile_id, item_kind, item_id)
);
CREATE INDEX idx_ratings_recent ON ratings(profile_id, rated_at DESC);

-- Las recomendaciones, «Tendencias» y el Top 10 cruzan ids de TMDB con el catálogo: sin
-- índice, cada búsqueda recorría la tabla entera (regla R4).
CREATE INDEX IF NOT EXISTS idx_movies_tmdb ON movies(tmdb_id);
CREATE INDEX IF NOT EXISTS idx_series_tmdb ON series(tmdb_id);

-- La cola de salida a la nube solo admitía perfiles, favoritos y progreso: sin esto las
-- valoraciones nunca habrían llegado a la nube (lo pilló el test de sincronización).
-- SQLite no cambia un CHECK en sitio: se rehace la tabla con lo que hubiera pendiente.
CREATE TABLE sync_outbox_new (
  id              INTEGER PRIMARY KEY,
  target          TEXT NOT NULL UNIQUE,
  collection      TEXT NOT NULL CHECK (collection IN ('profiles','favorites','playback','ratings')),
  payload         TEXT NOT NULL,
  revision        INTEGER NOT NULL DEFAULT 1,
  attempts        INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL,
  last_error      TEXT,
  created_at      INTEGER NOT NULL
);
INSERT INTO sync_outbox_new (id, target, collection, payload, revision, attempts, next_attempt_at, last_error, created_at)
  SELECT id, target, collection, payload, revision, attempts, next_attempt_at, last_error, created_at FROM sync_outbox;
DROP TABLE sync_outbox;
ALTER TABLE sync_outbox_new RENAME TO sync_outbox;
CREATE INDEX idx_sync_outbox_due ON sync_outbox(next_attempt_at);
