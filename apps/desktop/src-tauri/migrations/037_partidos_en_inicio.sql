-- F17 (2026-11-02, ver docs/DECISIONS.md): qué partidos salen en la portada de Inicio,
-- por perfil y con la cuenta: `mine` (solo sus equipos), `mine_and_big` (sus equipos y los
-- grandes: Madrid, Barça, Atlético, Champions...; lo de serie) o `never`.
ALTER TABLE profiles ADD COLUMN sports_home TEXT NOT NULL DEFAULT 'mine_and_big';
