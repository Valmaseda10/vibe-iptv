-- Modo LAN (F10): el servidor solo sirve a dispositivos fuera de 127.0.0.1 si están
-- emparejados y `app_settings.lan_enabled` vale '1'. Ver `docs/DECISIONS.md` para el
-- razonamiento completo del diseño (código de 6 dígitos de un solo uso, token por
-- dispositivo, revocable).

-- Ajustes de la app que no encajan en ninguna tabla existente. Clave/valor a propósito:
-- el único uso hoy es `lan_enabled`, y así no hace falta una migración nueva por cada
-- ajuste futuro que no merezca su propia columna.
CREATE TABLE app_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- El código de emparejamiento en sí (6 dígitos, de un solo uso, expira a los pocos
-- minutos) vive solo en memoria (`util::pairing::PairingState`): no tiene sentido
-- sobrevivir a un reinicio de la app, y así no hay que limpiar códigos caducados de la
-- base de datos. Lo que sí es permanente es el dispositivo ya emparejado.
CREATE TABLE paired_devices (
  id            INTEGER PRIMARY KEY,
  device_name   TEXT NOT NULL,
  -- SHA-256 del token, igual que `profiles.pin_hash`: el token en claro no se guarda en
  -- ningún sitio después de dárselo al dispositivo una vez (regla R5, mismo criterio
  -- que credenciales de proveedor aunque esto no sea un dato del proveedor).
  token_hash    TEXT NOT NULL UNIQUE,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER,
  revoked_at    INTEGER
);
CREATE INDEX idx_paired_devices_token ON paired_devices(token_hash);
