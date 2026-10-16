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

- Todas las rutas son relativas a `http://<host>:<puerto>` — el puerto es efímero
  (regla R2), se pide al PC durante el emparejamiento (ver más abajo) o, para la propia
  app de escritorio, con el comando Tauri `get_server_port`.
- JSON en request y response salvo `GET /stream/*` (vídeo) e `GET /img/*` (imágenes).
- IDs son siempre el id interno de VIBE IPTV (autoincremental de SQLite), nunca el id
  del proveedor. El cliente no necesita saber nada del proveedor: Xtream, M3U, todo se
  ve igual desde aquí.
- Paginación por cursor: `after=<último id visto>`, nunca offset (regla R4). La
  respuesta trae `next_cursor` — `null` cuando ya no hay más páginas.
- Ningún endpoint de catálogo devuelve nunca una URL de proveedor (regla R5): la
  reproducción siempre pasa por `/stream/*`, que resuelve la URL real internamente.
- **Local vs LAN**: todo lo de aquí funciona igual en `127.0.0.1` sin autenticación
  ninguna. Desde fuera de loopback, hace falta que el modo LAN esté activado y llevar
  `Authorization: Bearer <token>` — ver la sección **Modo LAN** al final; el resto del
  documento no repite esto en cada endpoint.

---

## Catálogo

### `GET /api/home`
Filas de la pantalla de inicio: continuar viendo, siguiente episodio pendiente,
películas recientes. Sin parámetros.

### `GET /api/live/categories`
Categorías de canales en directo. `[{ id, name }]`.

### `GET /api/live/channels`
Página de canales. Parámetros: `category_id`, `q` (búsqueda por nombre), `after`,
`limit` (máx. 100, por defecto 40). Devuelve `{ items: ChannelSummary[], next_cursor }`,
con `ChannelSummary = { id, display_name, logo_url, category_id, has_catchup }`.

### `GET /api/movies`
Página de películas. Parámetros: `genre`, `year`, `quality`, `sort`
(`title`|`year`|`rating`, por defecto lo más reciente), `after`, `limit`.

### `GET /api/movies/:id`
Ficha completa: título, sinopsis, portada, fondo, géneros, y `sources` (una entrada por
proveedor/calidad — varias si la misma película viene de más de una fuente). `404` si
no existe.

### `GET /api/series` / `GET /api/series/:id`
Igual que películas; el detalle trae `episodes[]` (temporada, número, título, duración).

### `GET /api/search?q=`
Búsqueda instantánea (FTS5). `{ channels: [], movies: [], series: [] }`, cada uno hasta
20 resultados. `q` vacío o solo símbolos da listas vacías, no error.

---

## Guía EPG

### `GET /api/epg/now?channel_ids=1,2,3`
Programa actual y siguiente de cada canal pedido (ids internos, coma-separados).
`[{ channel_id, current: {title, subtitle, start_ts, stop_ts} | null, next: ... | null }]`.

### `GET /api/epg/grid?channel_ids=1,2,3&from=<epoch>&to=<epoch>`
Todos los programas de esos canales que se solapan con `[from, to)`. `400` si
`to <= from`.

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
duration_s, audio_tracks, subtitle_tracks }`. `tier: null` significa "nunca se ha
reproducido, no hay sonda todavía", y entonces las listas de pistas van vacías: no se
sabe qué pistas tiene un fichero hasta abrirlo de verdad. `subtitle_tracks` solo lista
las de texto (SRT, ASS, mov_text, WebVTT); las de imagen (PGS, DVD) no se ofrecen.

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

## Control parental

- `GET /api/parental/status` → `{ has_pin, unlocked }`.
- `POST /api/parental/pin` con `{ pin: string | null }` (null quita el PIN).
- `POST /api/parental/unlock` con `{ pin }`.
- `POST /api/parental/lock`.

El desbloqueo es por sesión de la app (nunca se persiste); un cliente LAN que abra su
propia sesión de reproducción probablemente necesite su propio desbloqueo — esto no
está resuelto todavía por dispositivo, ver deuda técnica en `docs/DECISIONS.md`.

## Salud e imágenes

- `GET /api/health` → `{ status, version }`.
- `GET /img/:size/:image_id` — `size` es `w342`, `w500` o `w1280`; imágenes de TMDB ya
  cacheadas en disco.

---

## Modo LAN (F10)

Todo lo de arriba, más lo siguiente, sirve exactamente igual sea local o remoto — la
diferencia solo la impone el middleware de acceso (ver `server::lan`), no rutas
distintas.

### `GET /api/lan/status`
`{ enabled: bool, devices: [{ id, device_name, created_at, last_seen_at, revoked }],
local_address: string | null }`. `local_address` es la IP de este PC en la red local
(mejor esfuerzo; `null` si no se pudo determinar) — no hay descubrimiento automático
todavía, el cliente necesita saber a qué IP conectarse de antemano.

### `POST /api/lan/enable` / `POST /api/lan/disable`
Sin cuerpo. `204`. Actúa al instante, sin reiniciar la app.

### `POST /api/lan/pairing/start`
Lo llama el PC (el dispositivo que ya tiene la app) para generar un código de
emparejamiento. Sin cuerpo. `{ code: "123456", expires_in_s: 300 }`. Un código nuevo
invalida cualquiera pendiente; una tecleada equivocada en el otro dispositivo no lo
quema (se puede reintentar hasta que caduque).

### `POST /api/lan/pair`
Lo llama el dispositivo **nuevo**, sin token todavía — así consigue el primero. Único
endpoint que no exige `Authorization` aunque sí exige que el modo LAN esté activado.

Cuerpo: `{ code: string, device_name: string }`. Respuesta: `{ device_id, token }`.

**El `token` se manda una sola vez.** No hay forma de recuperarlo después (solo se
guarda su hash) — si el cliente lo pierde, hay que revocar el dispositivo y emparejarlo
de nuevo. Guárdalo de forma segura en el dispositivo (no en texto plano en un fichero
de configuración legible por cualquier app, si la plataforma lo permite).

Errores: `403` si el modo LAN está desactivado; `401` si el código no coincide o
caducó.

### `POST /api/lan/devices/:id/revoke`
Sin cuerpo. `204`. Revoca el token de ese dispositivo con efecto inmediato (la siguiente
petición suya devuelve `401`). No hay "des-revocar": hay que volver a emparejarlo.

### Autenticación del resto de rutas

Toda petición que llegue por una interfaz que no sea loopback:

1. Si el modo LAN está desactivado → `403`.
2. Si no lleva `Authorization: Bearer <token>` (y no es `POST /api/lan/pair`) → `401`.
3. Si el token no corresponde a ningún dispositivo activo (no emparejado, o revocado) →
   `401`.
4. En cualquier otro caso, la petición sigue exactamente igual que si viniera de
   loopback — el mismo `StreamGuard`, la misma base de datos, el mismo control parental.

No hay "roles": todo dispositivo emparejado ve todo lo que ve la propia app de
escritorio.

### StreamGuard y varios dispositivos

No existe un StreamGuard "por dispositivo": es el mismo objeto de este proceso para
absolutamente todo el mundo, local o LAN (ver `server::lan`, comentario de cabecera).
Si un dispositivo LAN pide directo mientras la propia app de escritorio está
reproduciendo algo, gana el que pide más tarde — exactamente la misma regla que ya
regía entre pestañas o zapping rápido dentro de la propia app (regla R1: una sola
conexión de stream activa, sea cual sea el origen de la petición).
