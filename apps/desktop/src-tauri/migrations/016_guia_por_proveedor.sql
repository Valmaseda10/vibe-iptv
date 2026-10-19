-- La guía de cada proveedor por separado (F13). Antes, sincronizar la guía de un
-- proveedor borraba la de todos: con dos proveedores solo quedaba la del último. Las
-- filas que ya había se quedan sin proveedor (NULL) hasta la próxima sincronización de
-- cualquiera, que las sustituye; mientras, se siguen mostrando.
ALTER TABLE epg_programmes ADD COLUMN provider_id INTEGER REFERENCES providers(id) ON DELETE CASCADE;
CREATE INDEX idx_epg_provider ON epg_programmes(provider_id);
