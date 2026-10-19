-- Recordatorios de programa (F13): "avísame cuando empiece". Locales, por perfil: no
-- viajan a la nube (F11 sincroniza perfiles, favoritos y progreso, no esto).
CREATE TABLE reminders (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  channel_id  INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  start_ts    INTEGER NOT NULL,
  stop_ts     INTEGER NOT NULL,
  title       TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (profile_id, channel_id, start_ts)
);
