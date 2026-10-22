-- Fuera el modo LAN de F10 (decidido con el usuario el 2026-10-22, ver docs/DECISIONS.md):
-- nunca llegó a funcionar (el servidor solo escucha en 127.0.0.1) y ningún cliente lo
-- necesita, porque el iPhone y el futuro Fire TV llevan su propio núcleo. `app_settings`
-- se queda: la usa la nube de cuentas.
DROP INDEX IF EXISTS idx_paired_devices_token;
DROP TABLE IF EXISTS paired_devices;
DELETE FROM app_settings WHERE key = 'lan_enabled';
