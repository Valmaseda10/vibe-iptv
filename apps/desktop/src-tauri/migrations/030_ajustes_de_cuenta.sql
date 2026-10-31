-- Ajustes de la cuenta en la nube (2026-10-31, ver docs/DECISIONS.md): hoy, la clave de
-- TMDB (`sync::tmdb_key`). La cola de salida solo admitía perfiles, favoritos, progreso y
-- valoraciones; SQLite no cambia un CHECK en sitio, así que se rehace la tabla con lo
-- que hubiera pendiente (igual que en 021). El valor de la clave nunca se guarda aquí.
CREATE TABLE sync_outbox_new (
  id              INTEGER PRIMARY KEY,
  target          TEXT NOT NULL UNIQUE,
  collection      TEXT NOT NULL CHECK (collection IN ('profiles','favorites','playback','ratings','settings')),
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
