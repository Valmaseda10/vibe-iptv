-- «Novedades» y el orden «Recientes» del catálogo ordenan por fecha de alta en cada visita:
-- sin índice recorrían y ordenaban la tabla entera (regla R4). Auditoría del 2026-10-30.
CREATE INDEX IF NOT EXISTS idx_movies_added ON movies(added_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_series_added ON series(added_at DESC, id DESC);
