-- Las series nunca guardaron su categoría del proveedor, a diferencia de canales y
-- películas: por eso el control parental por perfil (F "perfiles Netflix") no podía
-- ocultarlas en categorías de adultos. Las series ya sincronizadas quedan con
-- category_id NULL hasta la próxima sincronización, que es cuando se les asigna.
ALTER TABLE series_sources ADD COLUMN category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL;
