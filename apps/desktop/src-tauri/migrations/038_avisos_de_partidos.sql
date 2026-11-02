-- 2026-11-02 (ver docs/DECISIONS.md): seguir un equipo ya no avisa solo de sus partidos.
-- Los avisos van aparte, por perfil y con la cuenta, y vienen apagados: seguir sirve para
-- verlos en la portada de Inicio y destacados en Deportes.
ALTER TABLE profiles ADD COLUMN sports_notify INTEGER NOT NULL DEFAULT 0;
