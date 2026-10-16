-- Marcado manual de categorías para adultos (F12). Hasta ahora lo decidía solo una
-- heurística por nombre en cada sincronización ("XXX", "+18"...): si fallaba, un perfil
-- infantil veía la categoría y no había forma de corregirlo. NULL = automático; 0/1 =
-- decisión del usuario, que la sincronización respeta.
ALTER TABLE categories ADD COLUMN is_adult_manual INTEGER;
