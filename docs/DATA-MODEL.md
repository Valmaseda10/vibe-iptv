# Modelo de datos — VIBE IPTV

SQLite en el directorio de datos de la aplicación. `WAL` activado, `foreign_keys` on.
Migraciones numeradas en `src-tauri/migrations/NNN_descripcion.sql`, aplicadas al
arrancar. Nunca se edita una migración ya publicada; se añade otra.

---

## Esquema

```sql
-- ─────────────── Fuentes ───────────────
CREATE TABLE providers (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  kind            TEXT NOT NULL CHECK (kind IN ('xtream','m3u')),
  base_url        TEXT NOT NULL,
  username        TEXT,                 -- la contraseña vive en el keychain
  credential_ref  TEXT,                 -- clave de búsqueda en el keychain
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
  display_name   TEXT NOT NULL,        -- limpio: sin "ES |", sin "FHD"
  logo_url       TEXT,
  epg_channel_id TEXT,                 -- casa con epg_programmes.channel_id
  quality        TEXT,                 -- SD | HD | FHD | 4K
  language       TEXT,                 -- ES | EN | LAT ...
  has_catchup    INTEGER NOT NULL DEFAULT 0,
  catchup_days   INTEGER DEFAULT 0,
  sort_order     INTEGER,
  UNIQUE (provider_id, remote_id)
);
CREATE INDEX idx_channels_cat ON channels(category_id, sort_order);
CREATE INDEX idx_channels_epg ON channels(epg_channel_id);

-- ─────────────── Películas ───────────────
-- Un "title" agrupa varias "sources" del mismo contenido en distintas calidades.
CREATE TABLE movies (
  id           INTEGER PRIMARY KEY,
  clean_title  TEXT NOT NULL,
  year         INTEGER,
  match_key    TEXT NOT NULL,          -- slug(clean_title)+year, para agrupar
  tmdb_id      INTEGER,
  tmdb_state   TEXT NOT NULL DEFAULT 'pending'
                 CHECK (tmdb_state IN ('pending','matched','not_found','error')),
  overview     TEXT,
  poster_path  TEXT,
  backdrop_path TEXT,
  rating       REAL,
  runtime_min  INTEGER,
  genres       TEXT,                   -- JSON array de ids de género
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
  container    TEXT,                   -- mkv | mp4 | ts
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
  detail_synced_at INTEGER,            -- get_series_info es caro: se hace bajo demanda
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
  channel_id  TEXT NOT NULL,           -- id de XMLTV, no FK
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
  play_tier   INTEGER,                 -- 1, 2 o 3: qué nivel de la cascada funcionó
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
  stream_key  TEXT PRIMARY KEY,        -- hash de la URL sin credenciales
  video_codec TEXT, audio_codec TEXT, container TEXT,
  play_tier   INTEGER NOT NULL,
  probed_at   INTEGER NOT NULL
);
```

---

## Normalización de títulos

Es la pieza que decide si el catálogo parece Netflix o parece un `ls`. Implementar en
`catalog/normalize.rs` con tests exhaustivos sobre casos reales anonimizados.

### Orden de las reglas

1. **Prefijos de idioma o país**: `ES |`, `[ES]`, `ES:`, `LAT -`, `EN |`, `4K -`,
   `VIP |`, `#`, y variantes con espacios. Se extrae a `language` y se elimina.
2. **Marcas de calidad**: `4K`, `UHD`, `FHD`, `HD`, `SD`, `1080p`, `720p`, `HDR`,
   `DOLBY`. Se extrae a `quality` y se elimina. La primera coincidencia gana.
3. **Año**: `(2023)`, `[2023]`, ` 2023` al final. Se extrae a `year`.
   Cuidado: solo se acepta entre 1900 y el año actual más uno, para no confundirlo con
   títulos que llevan números (`2012`, `1917`, `Blade Runner 2049`). Si el número es
   también parte plausible del título, se conserva y se marca `year` como nulo.
4. **Etiquetas de release**: `WEB-DL`, `BluRay`, `HDTV`, `x264`, `x265`, `HEVC`,
   `DUAL`, `CASTELLANO`, `SUBTITULADO`, `MULTI`, `AC3`, `DTS`.
5. **Patrón de episodio**: `S01E02`, `1x02`, `T1 E2`, `Temporada 1 Capitulo 2`.
   Solo en fuentes M3U, donde no hay estructura de serie. Se extraen temporada y número.
6. **Limpieza final**: separadores sobrantes al principio y al final, espacios dobles,
   puntos y guiones bajos convertidos en espacios.

### Clave de agrupación

```
match_key = slug(clean_title_sin_articulos) + "-" + (year ?? "0000")
```

Donde `slug` pasa a minúsculas, quita diacríticos y deja `[a-z0-9-]`. Dos entradas con
la misma `match_key` son el mismo título en distinta calidad y se agrupan bajo una
única ficha.

Riesgo conocido: dos películas distintas del mismo año con el mismo nombre se fusionan.
Es aceptable, y se corrige después con el `tmdb_id` cuando el emparejamiento acierta.

### Emparejamiento con TMDB

- Consulta `search/movie` o `search/tv` con `query` y `year`, en español, con respaldo a
  inglés si no hay resultados.
- Se acepta el primer resultado si la similitud del título supera 0,85 y el año coincide
  con un margen de un año. Si no, `tmdb_state = 'not_found'`.
- **Los fallos también se cachean.** Sin eso, cada sincronización repite miles de
  consultas inútiles.
- Máximo 5 peticiones concurrentes. Ante un 429, espera exponencial.
