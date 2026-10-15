-- Carátula que da el propio proveedor (`stream_icon` de las películas, `cover` de las
-- series), guardada ya redactada (regla R5). Se usa cuando TMDB no tiene póster: sin
-- clave de API, o título que TMDB no encuentra. Al leer, `COALESCE(poster_path,
-- provider_poster_url)`: TMDB manda cuando lo tiene. Columna aparte en vez de reutilizar
-- `poster_path` para no mezclar una URL absoluta con las rutas de TMDB que sirve `/img`.
ALTER TABLE movies ADD COLUMN provider_poster_url TEXT;
ALTER TABLE series ADD COLUMN provider_poster_url TEXT;
