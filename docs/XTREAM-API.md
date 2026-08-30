# Referencia de proveedor: Xtream Codes y M3U

Documento de consulta para la implementación de `sources/`. **Los paneles Xtream varían
entre versiones y modificaciones.** Trata todos los campos como opcionales, valida los
tipos y no revientes si falta uno. Varios paneles devuelven números como cadenas.

---

## Xtream Codes

Base: `http://HOST:PUERTO/player_api.php?username=U&password=P`

### Información de cuenta
Sin parámetro `action`. Devuelve `user_info` y `server_info`.

Campos que importan de `user_info`: `auth` (1 si es válido), `status` (`Active`,
`Expired`, `Banned`), `exp_date` (epoch en segundos, como cadena, o nulo si no caduca),
`max_connections`, `active_cons`.

**`max_connections` es el dato más importante de toda la integración.** Se guarda en
`providers` y condiciona el StreamGuard. Si viene vacío o no numérico, se asume 1.

### Listados

| `action` | Devuelve |
|---|---|
| `get_live_categories` | `category_id`, `category_name`, `parent_id` |
| `get_vod_categories` | ídem |
| `get_series_categories` | ídem |
| `get_live_streams` | canales; admite `&category_id=` |
| `get_vod_streams` | películas; admite `&category_id=` |
| `get_series` | series, sin episodios |
| `get_series_info&series_id=X` | `seasons` y `episodes` agrupados por temporada |
| `get_vod_info&vod_id=X` | ficha con `info` (duración, códecs) y `movie_data` |
| `get_short_epg&stream_id=X&limit=N` | próximos programas, base64 en `title` y `description` |
| `get_simple_data_table&stream_id=X` | EPG del día completo del canal |

Campos frecuentes en `get_live_streams`: `stream_id`, `name`, `stream_icon`,
`epg_channel_id`, `category_id`, `tv_archive` (1 si hay catch-up),
`tv_archive_duration` (días), `num` (orden).

Campos frecuentes en `get_vod_streams`: `stream_id`, `name`, `stream_icon`,
`container_extension`, `category_id`, `added` (epoch), `rating`, y a veces `tmdb_id`.
**Si el proveedor da `tmdb_id`, úsalo y sáltate la búsqueda por título.**

En `get_series`: `series_id`, `name`, `cover`, `plot`, `genre`, `releaseDate`,
`last_modified`. En `get_series_info`, cada episodio trae `id`, `episode_num`,
`title`, `container_extension` e `info` con `duration_secs` y `movie_image`.

> `get_series_info` es una petición por serie. Con 3.000 series eso son 3.000
> peticiones. **No se hace durante la sincronización general**: se hace bajo demanda al
> abrir la ficha de la serie, y se cachea en `series_sources.detail_synced_at` con
> caducidad de siete días.

### URLs de reproducción

```
Directo    http://HOST:PUERTO/live/U/P/{stream_id}.ts        (o .m3u8)
Película   http://HOST:PUERTO/movie/U/P/{stream_id}.{container_extension}
Episodio   http://HOST:PUERTO/series/U/P/{episode_id}.{container_extension}
```

Algunos paneles antiguos sirven el directo sin el segmento `/live/`. Si la primera
forma devuelve 404, se reintenta sin él, una sola vez, y el resultado se recuerda por
proveedor.

Preferir `.ts` sobre `.m3u8` en directo: menos latencia de arranque y menos peticiones,
y el nivel 2 de la cascada lo maneja bien.

### Catch-up y timeshift

```
http://HOST:PUERTO/streaming/timeshift.php?username=U&password=P
    &stream={stream_id}&start=YYYY-MM-DD:HH-MM&duration={minutos}
```

Forma alternativa en paneles más nuevos:

```
http://HOST:PUERTO/timeshift/U/P/{duracion_min}/{YYYY-MM-DD:HH-MM}/{stream_id}.ts
```

Solo se ofrece en la interfaz si `tv_archive = 1`, y limitado a `tv_archive_duration`
días hacia atrás.

### EPG completo

```
http://HOST:PUERTO/xmltv.php?username=U&password=P
```

Puede venir comprimido con gzip y pesar decenas de megas. Se descarga a un fichero
temporal y se parsea desde disco en streaming, nunca en memoria.

### Cabeceras

Muchos proveedores rechazan peticiones sin un `User-Agent` de reproductor conocido.
Se envía uno configurable por proveedor, con `VLC/3.0.20 LibVLC/3.0.20` por defecto,
y se siguen las redirecciones. Tiempo de espera generoso en directo: los servidores
tardan en arrancar el flujo.

---

## Listas M3U

```
#EXTM3U x-tvg-url="http://host/xmltv.php?username=U&password=P"
#EXTINF:-1 tvg-id="canal.es" tvg-name="Canal ES" tvg-logo="http://..." group-title="ESPAÑA | GENERAL",ES | Canal HD
http://host:puerto/live/U/P/12345.ts
```

- El nombre visible es lo que va **después de la última coma**, no `tvg-name`.
- `x-tvg-url` en la primera línea da la fuente de EPG. Aprovéchala.
- La clasificación se hace por la ruta de la URL: `/live/`, `/movie/`, `/series/`. Si la
  URL no lo indica, se recurre a heurística sobre `group-title`, buscando términos como
  `VOD`, `PELICULA`, `MOVIE`, `SERIE`.

### Detección de credenciales embebidas

Un enlace de la forma:

```
http://HOST:PUERTO/get.php?username=U&password=P&type=m3u_plus&output=ts
```

es en realidad un proveedor Xtream. Al añadir la fuente, la app detecta este patrón,
prueba `player_api.php` con esas credenciales y, si responde, propone al usuario
convertirla en fuente Xtream. Merece la pena: se ganan las series estructuradas, el
catch-up y el dato de `max_connections`.

**Nunca mantener activas la variante M3U y la Xtream del mismo proveedor a la vez.**
Cuentan como dos conexiones.

---

## Errores típicos y qué significan

| Síntoma | Causa habitual |
|---|---|
| `auth: 0` en `player_api.php` | Credenciales incorrectas, o falta el `User-Agent` |
| 512 o 456 como código HTTP | Límite de conexiones alcanzado |
| El stream corta a los pocos segundos | Otra sesión abierta con la misma cuenta |
| Timeout solo en directo | Normal en el primer arranque; sube el timeout, no reintentes agresivamente |
| 404 en `/live/` pero la lista funciona | Panel antiguo sin ese segmento en la ruta |
| XMLTV vacío | El proveedor exige `User-Agent`, o el EPG va en un dominio distinto |
