-- 2026-11-02 (ver docs/DECISIONS.md): logos de canal de verdad. La mitad de los logos que
-- da el proveedor apuntan a webs caídas; el núcleo busca uno que funcione entre varias
-- fuentes, lo comprueba y lo guarda en el aparato (`catalog::logos`).

-- El logo de cada canal según la guía XMLTV (`<channel><icon src>`).
CREATE TABLE epg_channel_icons (
  provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  channel_id  TEXT NOT NULL,
  icon_url    TEXT NOT NULL,
  PRIMARY KEY (provider_id, channel_id)
);

-- Directorio abierto de iptv-org (dominio público): nombre normalizado → logo.
CREATE TABLE logo_directory (
  name_key  TEXT PRIMARY KEY,
  logo_url  TEXT NOT NULL,
  source_id TEXT NOT NULL,
  country   TEXT
);

-- Lo que se sabe de cada imagen candidata: si se pudo bajar y dónde se guardó.
CREATE TABLE logo_checks (
  url        TEXT PRIMARY KEY,
  ok         INTEGER NOT NULL,
  file       TEXT,
  checked_at INTEGER NOT NULL
);

-- El logo elegido a mano para un canal (por su nombre: vale para todas sus versiones).
CREATE TABLE channel_logo_choices (
  name_key   TEXT PRIMARY KEY,
  url        TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
