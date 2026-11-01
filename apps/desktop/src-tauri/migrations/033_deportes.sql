-- F17, parte 3 (2026-11-01, ver docs/DECISIONS.md): deportes.
--
-- «Qué hay hoy» en todos los canales a la vez: la guía por hora de inicio.
CREATE INDEX idx_epg_start ON epg_programmes(start_ts);

-- Equipos (o jugadores) y competiciones que sigue cada perfil: sus partidos avisan solos y
-- los importantes salen en la portada de Inicio. `key` es la de `catalog::sports`
-- («real-madrid», «champions», o el nombre normalizado de quien salga en la guía).
CREATE TABLE sport_follows (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('team','competition')),
  key         TEXT NOT NULL,
  label       TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (profile_id, kind, key)
);
