-- F17, parte 4 (2026-11-01, ver docs/DECISIONS.md): avisos de episodios nuevos.
--
-- El último episodio que se conocía de cada serie seguida, por perfil: hay uno nuevo
-- cuando el más alto (temporada, número) pasa de aquí. Comparar episodios y no la fecha
-- del proveedor: hay paneles que retocan la ficha sin añadir nada, o vuelven a subir el
-- mismo episodio con otro id.
CREATE TABLE new_episode_marks (
  profile_id  INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  series_id   INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
  season      INTEGER NOT NULL,
  number      INTEGER NOT NULL,
  checked_at  INTEGER NOT NULL,
  PRIMARY KEY (profile_id, series_id)
);
