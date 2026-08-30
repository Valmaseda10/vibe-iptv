-- Esquema inicial. Ver docs/DATA-MODEL.md — no se edita este fichero una vez publicado,
-- se añade otro con el siguiente número.

-- ─────────────── Fuentes ───────────────
CREATE TABLE providers (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  kind            TEXT NOT NULL CHECK (kind IN ('xtream','m3u')),
  base_url        TEXT NOT NULL,
  username        TEXT,
  credential_ref  TEXT,
  max_connections INTEGER DEFAULT 1,
  expires_at      INTEGER,
  enabled         INTEGER NOT NULL DEFAULT 1,
  last_sync_at    INTEGER,
  created_at      INTEGER NOT NULL
);

CREATE TABLE categories (
  id           INTEGER PRIMARY KEY,
  provider_id  INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  remote_id    TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('live','movie','series')),
  name         TEXT NOT NULL,
  is_adult     INTEGER NOT NULL DEFAULT 0,
  UNIQUE (provider_id, kind, remote_id)
);

-- ─────────────── Directo ───────────────
CREATE TABLE channels (
  id             INTEGER PRIMARY KEY,
  provider_id    INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  remote_id      TEXT NOT NULL,
  category_id    INTEGER REFERENCES categories(id) ON DELETE SET NULL,
  raw_name       TEXT NOT NULL,
  display_name   TEXT NOT NULL,
  logo_url       TEXT,
  epg_channel_id TEXT,
  quality        TEXT,
  language       TEXT,
  has_catchup    INTEGER NOT NULL DEFAULT 0,
  catchup_days   INTEGER DEFAULT 0,
  sort_order     INTEGER,
  UNIQUE (provider_id, remote_id)
);
CREATE INDEX idx_channels_cat ON channels(category_id, sort_order);
CREATE INDEX idx_channels_epg ON channels(epg_channel_id);

-- ─────────────── Películas ───────────────
CREATE TABLE movies (
  id           INTEGER PRIMARY KEY,
  clean_title  TEXT NOT NULL,
  year         INTEGER,
  match_key    TEXT NOT NULL,
  tmdb_id      INTEGER,
  tmdb_state   TEXT NOT NULL DEFAULT 'pending'
                 CHECK (tmdb_state IN ('pending','matched','not_found','error')),
  overview     TEXT,
  poster_path  TEXT,
  backdrop_path TEXT,
  rating       REAL,
  runtime_min  INTEGER,
  genres       TEXT,
  added_at     INTEGER,
  UNIQUE (match_key)
);

CREATE TABLE movie_sources (
  id           INTEGER PRIMARY KEY,
  movie_id     INTEGER NOT NULL REFERENCES movies(id) ON DELETE CASCADE,
  provider_id  INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  remote_id    TEXT NOT NULL,
  category_id  INTEGER REFERENCES categories(id) ON DELETE SET NULL,
  raw_title    TEXT NOT NULL,
  container    TEXT,
  quality      TEXT,
  language     TEXT,
  UNIQUE (provider_id, remote_id)
);
CREATE INDEX idx_movie_sources_movie ON movie_sources(movie_id);

-- ─────────────── Series ───────────────
CREATE TABLE series (
  id            INTEGER PRIMARY KEY,
  clean_title   TEXT NOT NULL,
  year          INTEGER,
  match_key     TEXT NOT NULL,
  tmdb_id       INTEGER,
  tmdb_state    TEXT NOT NULL DEFAULT 'pending',
  overview      TEXT,
  poster_path   TEXT,
  backdrop_path TEXT,
  rating        REAL,
  genres        TEXT,
  added_at      INTEGER,
  UNIQUE (match_key)
);

CREATE TABLE series_sources (
  id          INTEGER PRIMARY KEY,
  series_id   INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
  provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  remote_id   TEXT NOT NULL,
  raw_title   TEXT NOT NULL,
  detail_synced_at INTEGER,
  UNIQUE (provider_id, remote_id)
);

CREATE TABLE episodes (
  id                INTEGER PRIMARY KEY,
  series_source_id  INTEGER NOT NULL REFERENCES series_sources(id) ON DELETE CASCADE,
  remote_id         TEXT NOT NULL,
  season            INTEGER NOT NULL,
  number            INTEGER NOT NULL,
  title             TEXT,
  overview          TEXT,
  still_path        TEXT,
  duration_s        INTEGER,
  container         TEXT,
  air_date          TEXT,
  UNIQUE (series_source_id, remote_id)
);
CREATE INDEX idx_episodes_season ON episodes(series_source_id, season, number);

-- ─────────────── EPG ───────────────
CREATE TABLE epg_programmes (
  id          INTEGER PRIMARY KEY,
  channel_id  TEXT NOT NULL,
  start_ts    INTEGER NOT NULL,
  stop_ts     INTEGER NOT NULL,
  title       TEXT NOT NULL,
  subtitle    TEXT,
  description TEXT,
  category    TEXT,
  icon_url    TEXT
);
CREATE INDEX idx_epg_lookup ON epg_programmes(channel_id, start_ts, stop_ts);

-- ─────────────── Estado del usuario ───────────────
CREATE TABLE profiles (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  avatar      TEXT,
  pin_hash    TEXT,
  adult_ok    INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL
);

CREATE TABLE playback_state (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  item_kind   TEXT NOT NULL CHECK (item_kind IN ('movie','episode','channel')),
  item_id     INTEGER NOT NULL,
  position_s  INTEGER NOT NULL DEFAULT 0,
  duration_s  INTEGER,
  finished    INTEGER NOT NULL DEFAULT 0,
  played_at   INTEGER NOT NULL,
  play_tier   INTEGER,
  PRIMARY KEY (profile_id, item_kind, item_id)
);
CREATE INDEX idx_playback_recent ON playback_state(profile_id, played_at DESC);

CREATE TABLE favorites (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  item_kind   TEXT NOT NULL,
  item_id     INTEGER NOT NULL,
  added_at    INTEGER NOT NULL,
  PRIMARY KEY (profile_id, item_kind, item_id)
);

-- ─────────────── Búsqueda ───────────────
CREATE VIRTUAL TABLE search_index USING fts5(
  title, item_kind UNINDEXED, item_id UNINDEXED,
  tokenize = "unicode61 remove_diacritics 2"
);

-- ─────────────── Cachés ───────────────
CREATE TABLE tmdb_cache (
  query_hash  TEXT PRIMARY KEY,
  response    TEXT,
  fetched_at  INTEGER NOT NULL
);

CREATE TABLE probe_cache (
  stream_key  TEXT PRIMARY KEY,
  video_codec TEXT, audio_codec TEXT, container TEXT,
  play_tier   INTEGER NOT NULL,
  probed_at   INTEGER NOT NULL
);
