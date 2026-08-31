-- Perfiles de verdad (estilo Netflix): antes había como mucho un perfil implícito
-- ("Perfil", creado en silencio por `ensure_default_profile`). Ahora el usuario los crea,
-- edita y borra explícitamente, y opcionalmente se sincronizan contra Supabase.

-- `remote_id`: UUID del perfil en la tabla `public.profiles` de Supabase, si esta
-- cuenta ha iniciado sesión alguna vez y este perfil ya se ha sincronizado al menos una
-- vez. NULL mientras no haya cuenta, o para un perfil creado sin conexión todavía sin
-- subir. UNIQUE para poder hacer upsert por remote_id sin duplicar.
ALTER TABLE profiles ADD COLUMN remote_id TEXT;
CREATE UNIQUE INDEX idx_profiles_remote_id ON profiles(remote_id) WHERE remote_id IS NOT NULL;

-- Para decidir qué gana en un conflicto de sincronización (última escritura gana,
-- comparando esto contra `updated_at` de la fila remota).
ALTER TABLE profiles ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
UPDATE profiles SET updated_at = created_at WHERE updated_at = 0;
