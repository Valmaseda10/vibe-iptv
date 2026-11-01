-- F17, parte 3 (2026-11-01, ver docs/DECISIONS.md): «Mis equipos» y los recordatorios
-- viajan con la cuenta, cada perfil por separado. La cola de salida no admitía esas
-- colecciones; SQLite no cambia un CHECK en sitio: se rehace con lo pendiente (como 030).
CREATE TABLE sync_outbox_new (
  id              INTEGER PRIMARY KEY,
  target          TEXT NOT NULL UNIQUE,
  collection      TEXT NOT NULL CHECK (collection IN ('profiles','favorites','playback','ratings','settings','sport_follows','reminders')),
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
