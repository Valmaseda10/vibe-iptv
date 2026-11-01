-- F17, parte 1 (2026-11-01, ver docs/DECISIONS.md).
--
-- Cómo ha ido cada fuente de película o episodio: si arranca, cuánto tarda y si se ha
-- parado. Ordena las fuentes de una ficha (la que funciona primero) y decide a cuál se
-- pasa si una falla. Por el par del proveedor (`provider_id`, `remote_id`), que no cambia
-- al volver a sincronizar; nunca por URL (R5).
CREATE TABLE source_health (
  provider_id     INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  item_kind       TEXT NOT NULL CHECK (item_kind IN ('movie','episode')),
  remote_id       TEXT NOT NULL,
  successes       INTEGER NOT NULL DEFAULT 0,
  failures        INTEGER NOT NULL DEFAULT 0,
  slow_reports    INTEGER NOT NULL DEFAULT 0,
  last_start_ms   INTEGER,
  last_success_at INTEGER,
  last_failure_at INTEGER,
  last_slow_at    INTEGER,
  PRIMARY KEY (provider_id, item_kind, remote_id)
);

-- Tiempo visto de verdad, por perfil, día y hora local y título: lo que suma «Tu año en
-- VIBE». `device_id` es el aparato que lo vio (cada uno escribe solo lo suyo, y en la
-- nube se suman). `item_key` es la clave de la nube (`reads::item_key_for`), para que lo
-- visto en otro aparato con otro catálogo se pueda contar igual; `title` guarda el nombre
-- por si ese título ya no está en el catálogo.
CREATE TABLE watch_time (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  day         TEXT NOT NULL,
  hour        INTEGER NOT NULL CHECK (hour BETWEEN 0 AND 23),
  item_kind   TEXT NOT NULL CHECK (item_kind IN ('movie','episode','channel')),
  item_key    TEXT NOT NULL,
  title       TEXT NOT NULL,
  seconds     INTEGER NOT NULL DEFAULT 0,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (profile_id, device_id, day, hour, item_kind, item_key)
);
CREATE INDEX idx_watch_time_day ON watch_time(profile_id, day);
