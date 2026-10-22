# API HTTP local

Referencia de la API que expone el servidor axum embebido en la app de escritorio, para
el futuro cliente de Fire TV (F10 la documenta; el cliente en sí llega después, ver
`CLAUDE.md` sección 7). El WebView de la propia app de escritorio consume esta misma
API — no hay una API "interna" distinta de la que vería un cliente remoto.

No es una API pública ni versionada: cambia con la app. Este documento describe el
estado en el momento de F10; si algo no coincide con el código, gana el código.

Perfiles, favoritos y progreso también viven en la nube desde F11: un cliente de otra
plataforma los lee y escribe directamente en Firebase con el modelo de
`docs/CLOUD-SYNC.md`, no a través de esta API. Las rutas `/api/account/*` son solo para
la pantalla de cuenta del escritorio.

## Convenciones

- Todas las rutas son relativas a `http://127.0.0.1:<puerto>` — el puerto es efímero
  (regla R2) y la interfaz lo pide con el comando Tauri `get_server_port`. Solo la usa la
  propia app: nada escucha en la red.
- JSON en request y response salvo `GET /stream/*` (vídeo) e `GET /img/*` (imágenes).
- IDs son siempre el id interno de VIBE IPTV (autoincremental de SQLite), nunca el id
  del proveedor. El cliente no necesita saber nada del proveedor: Xtream, M3U, todo se
  ve igual desde aquí.
- Paginación por cursor: `after=<último id visto>`, nunca offset (regla R4). La
  respuesta trae `next_cursor` — `null` cuando ya no hay más páginas.
- Ningún endpoint de catálogo devuelve nunca una URL de proveedor (regla R5): la
  reproducción siempre pasa por `/stream/*`, que resuelve la URL real internamente.

---

## Catálogo

### `GET /api/home`
Filas de la pantalla de inicio: continuar viendo, siguiente episodio pendiente,
películas recientes, recomendadas para el perfil (`recommended`, `recommended_series` y
`because_rows`, las filas «Porque viste X»), tendencias, el Top 10 en España
(`top_movies_es`, `top_series_es`: lo más popular en plataformas en España según TMDB,
entre lo que tiene el proveedor) y filas por género. Lo que depende de TMDB sale vacío
sin clave. `profile_id`.

### `GET /api/live/categories`
Categorías de canales en directo. `[{ id, name }]`.

### `GET /api/live/channels`
Página de canales, por número del proveedor y luego id. Parámetros: `category_id`, `q`
(búsqueda por nombre), `limit` (máx. 100, por defecto 40) y, para la página siguiente,
`after` + `after_key` (los `next_cursor` y `next_cursor_key` de la anterior, tal cual).
Con `favorites=1`, solo los canales favoritos del perfil («★ Favoritos» del directo), en
el mismo orden. Con `around=<id de canal>` (el reproductor) no se pagina: devuelve la lista entera si cabe
en `limit` y, si no, una ventana con ese canal en medio que da la vuelta por los extremos
(tras el último, el primero), como el zapping; `next_cursor` va vacío.
Devuelve `{ items: ChannelSummary[], next_cursor, next_cursor_key }`, con
`ChannelSummary = { id, display_name, logo_url, category_id, has_catchup, number, quality, language }`
(`number` es el número del canal en el proveedor, o `null`; `quality` e `language` son
lo que traía el nombre, «HD» o «ES», o `null`).

### `GET /api/live/channels/:id/variants?profile_id=`
Las versiones de ese canal: los que tienen el mismo nombre una vez quitada la calidad
(«La 1 HD», «La 1 4K»), de cualquier proveedor, él incluido, de mejor a peor calidad.
Para el selector «Fuente» del reproductor. `ChannelSummary[]`, vacío si no existe.

### `GET /api/live/channels/by-number/:number?profile_id=&near=`
El canal con ese número (para teclearlo en el directo). Con varios proveedores el número
se repite: `near` (el canal que se está viendo) hace ganar al de su proveedor. `404` si no
hay ninguno o el perfil no puede verlo.

### `GET /api/live/recent?profile_id=&limit=`
Últimos canales que abrió el perfil, el más reciente primero (12 por defecto).

### `GET /api/browse/:page?profile_id=&seed=&avoid=`
Portada y filas de Películas (`movies`), Series (`series`) o la mezcla de Inicio (`home`),
como en Netflix (`catalog::browse`). `seed` es la semilla de la visita: la misma da la misma
pantalla y otra la cambia (portada, títulos de cada fila y orden de las filas de
descubrimiento). `avoid` son portadas recientes que no repetir (`movie:12,series:4`).
Devuelve `{ featured, rows, pending }`: `featured` (`BrowseFeatured`, con `reason`: «N.º 2 en
España hoy»...) o `null`; `rows` con `key` estable, `title`, `style` (`poster`, `top10` o
`continue`) e `items` (`BrowseItem`: `kind` movie/series/episode, `id`, `series_id`,
`clean_title`, `year`, `poster_path`, `subtitle`, `progress`, `is_new`). `pending` es
`true` mientras se calculan por primera vez las recomendaciones del perfil. En Películas y
Series la primera fila es «Seguir viendo» si hay algo a medias.

### `GET /api/movies`
Página de películas. Parámetros: `genre`, `year`, `quality`, `sort`
(`title`|`year`|`rating`, por defecto lo más reciente), `after`, `limit`.

### `GET /api/movies/:id`
Ficha completa: título, sinopsis, portada, fondo, géneros, y `sources` (una entrada por
proveedor/calidad — varias si la misma película viene de más de una fuente). `404` si
no existe.

### `GET /api/series` / `GET /api/series/:id`
Igual que películas; el detalle trae `episodes[]` (temporada, número, título, duración,
`watched`) y el episodio del botón «Ver/Continuar T·E»: `up_next_episode_id` y
`up_next_started` (si el perfil ya iba por la serie). La regla vive en el núcleo
(`reads::up_next_episode`): manda lo último que se tocó; empezado sin terminar, ese
mismo; terminado, el siguiente; tras el último o sin nada visto, el primero.

### `GET /api/search?q=&profile_id=`
Búsqueda instantánea (FTS5). `{ channels: [], movies: [], series: [] }`, cada uno hasta
20 resultados. `q` vacío o solo símbolos da listas vacías, no error. Lo que el perfil no
puede ver no sale.

### `GET /api/random?profile_id=`
"Sorpréndeme": `{ kind: "movie" | "series", id }` al azar entre lo que el perfil puede
ver, o `204` si no hay nada.

### Qué ve cada perfil
Todas las rutas de catálogo filtran por el perfil (`profile_id`): listas, búsqueda,
«Sorpréndeme», Inicio, similares y favoritos. Una ficha (`/api/movies/:id`,
`/api/series/:id`) o un stream de algo que el perfil no puede ver responde `403` con el
motivo. Un perfil con `max_age` solo ve títulos con clasificación por edad **conocida** y
no mayor; lo que no tiene clasificación no se le enseña nunca. Un `profile_id` que no
existe ve lo mínimo, nunca todo.

---

## Guía EPG

### `GET /api/epg/now?channel_ids=1,2,3`
Programa actual y siguiente de cada canal pedido (ids internos, coma-separados).
`[{ channel_id, current: {title, subtitle, start_ts, stop_ts} | null, next: ... | null }]`.

### `GET /api/epg/grid?channel_ids=1,2,3&from=<epoch>&to=<epoch>`
Todos los programas de esos canales que se solapan con `[from, to)`. `400` si
`to <= from`. La guía de cada canal es la de su proveedor.

### `GET /api/epg/search?profile_id=&q=&limit=`
Programas en emisión o futuros cuyo título contiene `q` (al menos dos letras), por orden
de comienzo: `[{ channel: ChannelSummary, title, subtitle, start_ts, stop_ts }]`. Un mismo
programa sale una vez por cada canal (o feed) que lo emite.

### Recordatorios
- `GET /api/reminders?profile_id=` → `[{ channel: ChannelSummary, title, start_ts, stop_ts }]`,
  los que aún no han terminado, el más próximo primero. Los terminados se borran solos.
- `POST /api/reminders` con `{ profile_id, channel_id, start_ts, stop_ts, title }`. `400`
  si el programa ya terminó.
- `DELETE /api/reminders?profile_id=&channel_id=&start_ts=`.

Son locales del PC, por perfil: no viajan a la nube. El aviso lo da cada cliente (la app
de escritorio lo enseña un minuto antes de que empiece).

---

## Reproducción

### `GET /stream/:kind/:id`
El vídeo en sí — `<video src>` apunta aquí directamente. `kind` es `channel`, `movie` o
`episode`. Devuelve directamente los bytes de vídeo (nivel 1: passthrough con Range;
nivel 2: fMP4 fragmentado) o un error (ver más abajo) si hace falta nivel 3. Parámetros:

| Parámetro | Para qué |
|---|---|
| `profile_id` | Obligatorio. De quién es la sesión (registro de nivel, progreso). |
| `source=<id>` | Una fuente concreta si la película tiene varias calidades. |
| `start=<segundos>` | Remux empezando en ese punto: así se busca en VOD de nivel 2. |
| `audio_track=<n>` | Índice relativo de pista de audio (de `tier.audio_tracks`). |
| `audio_passthrough=ac3,eac3` | Audio que el cliente decodifica él mismo: se copia en vez de convertirse a AAC. Solo se aceptan `ac3`, `eac3` y `dts`. |
| `subtitle_track=<n>` | Pista de subtítulos de texto (de `tier.subtitle_tracks`). Fuerza el nivel 2 aunque el fichero admitiera el 1; un índice que la sonda no conoce se ignora. |

### `GET /api/playback/subtitles/:kind/:id?track=<n>&start=<segundos>&from=<k>`
Frases de subtítulos que el remux en curso lleva extraídas (F12): `{ active, cues: [{
start, end, text }] }`. Salen del **mismo** ffmpeg que sirve el vídeo, a medida que
avanza, así que se piden cada poco y `from` es cuántas tiene ya el cliente. Los tiempos
están en la escala del vídeo servido: cero en el punto de `?start=`. `active: false` si
el stream en curso no es ese ítem con esa pista y ese `start` (p. ej. el usuario ya
buscó o cambió de contenido): las frases de esa sesión ya no existen. `text` puede
llevar las etiquetas de WebVTT (`<i>`, `<b>`).

### Pausar y rebobinar el directo
Mientras se ve un canal en directo por `/stream/channel/:id`, el núcleo lo graba a la vez
(mismo proceso, misma conexión con el proveedor) como HLS fMP4 con 30 min de ventana:

- `GET /stream/timeshift/channel/:id/live.m3u8` (y los `init.mp4` / `seg-*.m4s` que
  lista): la grabación, para un reproductor HLS. `404` si no hay grabación de ese canal.
- `POST /api/timeshift/:channel_id/keepalive`: el cliente sigue en el canal. **Hay que
  mandarla cada pocos segundos** mientras se esté en pausa o en la grabación: sin leer el
  directo ni señales de vida, la sesión se cierra a los 20 s y libera la conexión.
- `DELETE /api/timeshift/:channel_id`: el cliente se va; la conexión se libera ya.

En iPhone/iPad (sin ffmpeg) la grabación es un anillo de MPEG-TS crudo:

- `GET /stream/live-buffer/channel/:id?profile_id=&behind_s=<segundos>&session=<id>`: el
  directo desde la grabación, `behind_s` segundos por detrás (0: en directo). `session`
  identifica al reproductor abierto (una por apertura); si esa sesión ya se cerró,
  responde `410` sin abrir nada con el proveedor.
- `DELETE /api/live-buffer?channel_id=<id>&session=<id>`: el reproductor se cerró; la
  grabación termina, la conexión se libera ya y la sesión queda cerrada (VLC reconecta
  solo y, sin esto, volvía a abrir el directo). Con `channel_id`, solo termina la
  grabación si es de ese canal; sin él, la que haya.

Mientras la lista empieza en su primer trozo (`#EXT-X-MEDIA-SEQUENCE:0`, los primeros
30 min), sus segundos son los mismos que los del directo: pausar en el segundo 120 del
directo es seguir en el 120 de la grabación. Después, solo vale contar respecto al
borde de la lista. MP2/MP3 llegan como AAC en la grabación (MSE no los acepta).

### `GET /api/playback/last-error/:kind/:id`
Por qué falló el último intento de abrir ese stream (`kind` también puede ser
`catchup`): `{ status, message }`, o `204` si abrió bien. Existe porque un `<video>`
nunca ve el cuerpo de una respuesta de error, y volver a pedir el stream para leerlo
abriría otra sesión con el proveedor (R1).

### `GET /stream/catchup/:channel_id?start=<epoch>&duration_min=<min>`
Igual que el directo pero de un bloque pasado, solo si el canal anuncia catch-up
(`ChannelSummary.has_catchup`) y `start` cae dentro de lo que el proveedor guarda.
`404` si el canal no existe o no soporta catch-up; `400` si `start` está fuera de rango.

### `POST /stream/mpv/:kind/:id`
Lanza mpv en una ventana aparte (nivel 3) para contenido que el proxy no puede servir
en la ventana de la app. Responde `204` en cuanto mpv arranca (no cuando termina).

### `GET /api/playback/tier/:kind/:id`
Qué nivel de la cascada sirvió (o serviría) este ítem, según la sonda ya cacheada — no
abre ninguna conexión nueva. `{ tier: 1|2|3|null, video_codec, audio_codec, container,
duration_s, audio_tracks, subtitle_tracks, intro_start_s, intro_end_s }`. `tier: null` significa "nunca se ha
reproducido, no hay sonda todavía", y entonces las listas de pistas van vacías: no se
sabe qué pistas tiene un fichero hasta abrirlo de verdad. `subtitle_tracks` solo lista
las de texto (SRT, ASS, mov_text, WebVTT); las de imagen (PGS, DVD) no se ofrecen.
`intro_start_s`/`intro_end_s` son el capítulo de intro del fichero, solo si tiene uno con
ese nombre ("Intro", "Opening", "Cabecera"...): el cliente puede ofrecer "Saltar intro".
Nunca se deduce por tiempos.

### `GET /api/playback/binaries`
`{ ffmpeg: bool, ffprobe: bool, mpv: bool }` — si falta alguno, los endpoints que lo
necesitan fallan con un error claro en vez de colgarse.

### Errores de `/stream/*`
Mensajes en español, pensados para enseñarse tal cual (ver `server::stream::StreamError`):

| Estado | Significa |
|---|---|
| 404 | Contenido no encontrado, o canal sin catch-up |
| 400 | `kind` inválido, o ventana de catch-up fuera de rango |
| 402 | La cuenta del proveedor caducó |
| 502 | No se pudo conectar con el proveedor (límite de conexiones, caída, sin red — no se puede distinguir con certeza) |
| 503 | Falta un binario (ffmpeg/ffprobe/mpv) |
| 501 | El contenido necesita nivel 3 (mpv) — el cliente debería ofrecer "abrir con mpv" |

### Progreso de reproducción

- `GET /api/playback/progress?item_kind=movie&item_id=5` → `{ position_s, duration_s,
  finished }`, o `204` si nunca se ha reproducido.
- `POST /api/playback/progress` con `{ item_kind, item_id, position_s, duration_s }`.
  `item_kind` es `movie`, `episode` o `channel` (aunque el directo no llega a guardar
  progreso real — no tiene "continuar viendo").

---

## Favoritos

- `GET /api/favorites` → `[{ item_kind, item_id, title, poster_path }]`.
- `POST /api/favorites` con `{ item_kind, item_id }`.
- `DELETE /api/favorites?item_kind=movie&item_id=5`.

## Valoraciones

Por perfil, como en Netflix: `rating` -1 «No es para mí», 1 «Me gusta», 2 «Me encanta».
Solo películas y series. Viajan entre dispositivos por la nube y alimentan las
recomendaciones de Inicio.

- `GET /api/ratings?profile_id=` → `[{ item_kind, item_id, rating }]`, la más reciente primero.
- `POST /api/ratings` con `{ profile_id, item_kind, item_id, rating }`. `400` con otro valor.
- `DELETE /api/ratings?profile_id=&item_kind=movie&item_id=5`.

## Perfiles y control parental

El control parental es por perfil (sustituyó al PIN global de F8).

- `GET /api/profiles` → `[{ id, name, avatar, has_pin, adult_ok, max_age }]`.
  `adult_ok` es "ve categorías XXX"; `max_age` es la edad máxima de lo que ve (`null`,
  sin límite).
- `POST /api/profiles` y `PATCH /api/profiles/:id` con `{ name, avatar, adult_ok, max_age,
  adult_pin }`. Si el cambio deja ver más (activar `adult_ok`, subir o quitar `max_age`,
  o crear un perfil sin límite), hace falta `adult_pin`: el PIN de cualquier perfil sin
  límite de edad que tenga PIN. Sin él, `403`. Si ningún perfil así tiene PIN, no hace
  falta.
- `POST /api/profiles/:id/pin` con `{ pin, current_pin }` (`pin: null` lo quita). Si el
  perfil ya tiene PIN, `current_pin` tiene que ser ese o el de un adulto; si no, `401`.
- `POST /api/profiles/:id/enter` con `{ pin }`: comprueba el PIN al elegir perfil.
- `DELETE /api/profiles/:id`.

## Salud e imágenes

- `GET /api/health` → `{ status, version }`.
- `GET /img/:size/:image_id` — `size` es `w342`, `w500` o `w1280`; imágenes de TMDB ya
  cacheadas en disco.

---
