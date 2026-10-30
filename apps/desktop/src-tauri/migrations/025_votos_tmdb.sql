-- Número de votos de TMDB (2026-10-30, ver docs/DECISIONS.md): sin él, una nota de 9,5 con
-- tres votos parecía una obra maestra en «Aclamadas» o en la portada. Se rellena al
-- emparejar (la búsqueda lo trae) y, para lo ya emparejado, poco a poco por detrás.
ALTER TABLE movies ADD COLUMN vote_count INTEGER;
ALTER TABLE series ADD COLUMN vote_count INTEGER;
