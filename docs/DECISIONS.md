# Registro de decisiones

Una entrada por fase cerrada o por decisión que cambie el rumbo. Formato: fecha, qué,
por qué, y qué alternativa se descartó.

---

## 2026-08-30 — F0 Andamiaje

**Qué se ha construido.** Proyecto `apps/desktop` arrancado: Tauri 2 + React 19 +
TypeScript estricto + Vite en el frontend; Rust (tokio, axum, rusqlite, reqwest,
tracing, keyring, ts-rs) en el núcleo. Servidor axum que reserva un puerto efímero en
`127.0.0.1` de forma síncrona antes de arrancar Tauri y lo expone al frontend por el
comando `get_server_port`. `GET /api/health` responde con un tipo `HealthStatus`
exportado a `src/lib/api-types.ts` vía ts-rs. Helper `util::redact()` implementado con
7 tests, cubriendo userinfo, query Xtream (`get.php?username=...`), segmentos de ruta
(`/live/<user>/<pass>/id.ts`, también `/movie/` y `/series/`) y un respaldo por texto
cuando la cadena no es una URL válida. Estructura de carpetas de la sección 4 de
CLAUDE.md, con `sources/`, `catalog/`, `db/`, `media/` declarados y vacíos.

**Decisiones tomadas.**
- **Migraciones fuera de `db/`.** La sección 4 de CLAUDE.md decía que `db/` contenía
  "migraciones, queries, schema.sql", pero `docs/DATA-MODEL.md` ya fijaba
  `src-tauri/migrations/NNN_descripcion.sql` como ubicación. Se corrigió la sección 4
  de CLAUDE.md: `migrations/` como carpeta hermana de `src/`, y `db/` con solo
  `schema.rs` y `queries.rs`.
- **`rusqlite` sin feature `fts5` explícita.** La versión actual (0.40) ya no expone
  `fts5` como feature de Cargo: el build `bundled` compila SQLite con
  `-DSQLITE_ENABLE_FTS5` siempre activo. Mismo resultado funcional, `Cargo.toml` solo
  lleva `features = ["bundled"]`.
- **CORS abierto (`Access-Control-Allow-Origin: *`) en el servidor local.** El WebView
  sirve el frontend desde un origen distinto al del servidor axum
  (`http://localhost:1420` en dev, `tauri://localhost` empaquetado), así que sin CORS
  el `fetch` del frontend fallaba con "Failed to fetch" aunque el servidor respondiera
  bien (confirmado con `curl`/`Invoke-WebRequest` directo). Se añadió `tower-http` con
  `CorsLayer` permisivo. Aceptable porque el servidor solo escucha en 127.0.0.1 y no usa
  cookies ni sesión (R2); a revisar cuando se abra la LAN en F10.
- **Icono de aplicación placeholder.** Tauri exige `icons/icon.ico` para generar el
  recurso de Windows incluso en modo dev. Se generó un icono provisional (círculo rojo
  sobre fondo oscuro) con `System.Drawing`; el branding real queda pendiente de F10.
- **Rust y MSVC Build Tools instalados en la máquina de desarrollo.** No estaban
  presentes; se instaló `rustup` (toolchain estable 1.98) con autorización explícita.
  Visual Studio con el workload C++ ya estaba presente.

**Bug real encontrado y corregido durante la verificación:** en Windows, un
`std::net::TcpListener` clonado con `try_clone()` no hereda el modo no bloqueante del
original (es un atributo de Winsock por handle). El socket clonado que se pasaba al
`tokio::net::TcpListener::from_std` dentro de la tarea de fondo quedaba bloqueante,
así que `accept()` colgaba el hilo del runtime sin ningún error visible: el puerto
aparecía en `LISTEN` pero ninguna petición obtenía respuesta. Se corrigió llamando a
`set_nonblocking(true)` de nuevo sobre el socket ya clonado, dentro de `server::serve()`.

**Deuda técnica pendiente.**
- CORS permisivo sin restricción de origen (ver arriba); revisar en F10.
- Icono de aplicación provisional, sin identidad visual real.
- `keyring` y `ts-rs` están en `Cargo.toml` sin consumidores todavía (llegan en F1/F2 y
  F4 respectivamente); `util::redact()` tiene el mismo caso, con `#[allow(dead_code)]`
  documentado hasta que `sources::xtream`/`sources::m3u` lo usen en F1.

---

## 2026-08-30 — F2 Base de datos y sincronización

**Qué se ha construido.** Migración inicial (`001_schema_inicial.sql`) con el esquema
completo de `docs/DATA-MODEL.md`, embebida en el binario y aplicada una sola vez vía
tabla `_migrations`. `db::queries` con upserts tipados e idempotentes para proveedores,
categorías, canales, películas y series. `StreamGuard` real (semáforo con generación
para invalidar permisos viejos, `Drop` a prueba de pánico). `util::keychain` sobre
`keyring`. `catalog::sync::run` orquesta los 4 pasos de `docs/ARCHITECTURE.md` por lotes
de 500, se pausa (nunca cancela) si `StreamGuard::is_busy()`, es cancelable, y emite
progreso vía closure (los comandos lo reenvían como evento Tauri `sync://progress`).
Pantalla de Ajustes (`/settings`) funcional: alta de proveedor Xtream o M3U, validación
en vivo, detección de `get.php` disfrazado de Xtream, lista de proveedores con
resincronizar y barra de progreso. 69 tests.

**Decisiones tomadas.**
- **La URL de M3U es una credencial.** `providers.base_url` guarda la versión redactada
  de la URL para proveedores M3U; la URL real (con usuario/contraseña embebidos, si los
  hay) va al keychain bajo `credential_ref`, igual que la contraseña de Xtream. Xtream no
  cambia: su `base_url` nunca lleva credenciales.
- **`clean_title`/`match_key` son un placeholder** (título tal cual, sin año) hasta que
  F3 implemente `catalog::normalize`. Confirmado contra el proveedor real: un mismo show
  puede llegar con la calidad pegada al título (`"... 1080p"`), así que el placeholder
  agrupa peor que la normalización real, pero nunca revienta ni pierde datos.
- **`StreamGuard` es el semáforo completo ya, sin proceso real que matar** (eso es F6).
  Se implementó `acquire` como `async` desde ahora, aunque hoy no haya nada que esperar,
  para no romper la firma cuando F6 necesite esperar de verdad a que muera un proceso.
- **`rusqlite::Connection` viaja entre el hilo async y el pool de tareas bloqueantes de
  tokio** (patrón "ping-pong" con `spawn_blocking`) porque rusqlite es síncrono y
  `Source` es async. Se prefirió sobre `r2d2`/un pool de conexiones: un único escritor a
  la vez encaja con cómo sincroniza esta app.
- **Provider CRUD y sincronización van por comandos Tauri, no por HTTP**, hasta que F4
  construya `/api/*`. `src-tauri/src/commands.rs` es la capa fina que lo conecta todo.

**Bugs reales encontrados verificando contra el proveedor real del usuario y contra la
app de verdad (no en teoría):**
- **`keyring` sin la feature `windows-native` cae en un backend "mock" en Windows sin
  avisar.** Las contraseñas parecían guardarse (la llamada no fallaba) pero jamás se
  podían volver a leer (`NoEntry`). Sin este arreglo, la "seguridad" de R5 habría sido
  una ilusión: nunca se habría podido recuperar ninguna contraseña guardada. Corregido
  añadiendo `features = ["windows-native"]` a la dependencia.
- **`ts-rs` mapea `i64`/`u64` a `bigint`, pero Tauri serializa los comandos por JSON
  normal** (`JSON.parse`), así que en tiempo de ejecución esos campos son `number`, no
  `bigint` de verdad, pese a lo que decía el tipo generado. Corregido con
  `#[ts(type = "number")]` / `"number | null"` en todos los campos `i64`/`u64` expuestos
  (ids, contadores, timestamps epoch): todos caben sobrados en
  `Number.MAX_SAFE_INTEGER`.
- **`stream_icon` (y campos de texto similares) puede llegar como `""` en vez de ausente
  o `null`.** No estaba cubierto por los fixtures inventados de F1. Añadido
  `serde_helpers::non_empty_string_opt` y aplicado a `stream_icon`/`epg_channel_id`.

**Verificación real (no queda en el repo, solo aquí).** Con el proveedor Xtream real del
usuario: cuenta válida, 615 canales, 23.388 películas, 8.684 series, todo parseado sin
errores; detección de `get.php` correcta; sincronización completa a SQLite en disco:
31.754 ítems en 11,3 s (el criterio de aceptación pide menos de 60 s para 50.000).

**Deuda técnica pendiente.**
- Persistencia de la URL de reproducción de M3U entre reinicios — sigue sin resolver
  (heredado de F1, ver esa entrada).
- CORS permisivo e icono provisional — heredados de F0.
- Sin botón de cancelar sincronización en la interfaz (el motor sí es cancelable; falta
  la UI). Sin edición/baja de proveedores.
- La pantalla de Ajustes es funcional, no la versión visual definitiva — eso es F5.

---

## 2026-08-31 — F3 Normalización y TMDB

**Qué se ha construido.** `catalog::normalize`: las 6 reglas de `docs/DATA-MODEL.md` en
orden, con 40 tests (más de los 30 pedidos). `catalog::tmdb`: `TmdbClient` (búsqueda
es-ES con respaldo en-US, detalle directo por `tmdb_id`, 5 peticiones concurrentes,
reintento exponencial ante 429), similitud por Levenshtein normalizado, caché de
aciertos y fallos en `tmdb_cache`, y `run_matching_pass` recorriendo `movies`/`series`
en `pending`/`error`. `db::queries::upsert_movie/upsert_series/upsert_channel` ahora
reciben los campos ya normalizados en vez de calcular un placeholder. `GET
/img/:size/:image_id` sirve portadas/fondos con caché en disco, descargando de
`image.tmdb.org` si falta. Ajustes: campo de clave de API de TMDB (al keychain) y botón
"Buscar en TMDB". 133 tests en total.

**Decisiones tomadas.**
- **Solo se extrae año entre `()`/`[]`, nunca un año suelto al final.** "1917", "2012" y
  "Blade Runner 2049" son indistinguibles de un año-sufijo real usando solo texto, y en
  el catálogo real del usuario los títulos con año siempre usan paréntesis. Confirmada
  con el usuario antes de implementar.
- **`runtime_min` se queda sin rellenar.** `search/movie` de TMDB no lo trae; pedirlo
  exigiría una petición extra por película a `/movie/{id}`, duplicando la carga contra
  el límite de 5 concurrentes, y el criterio de aceptación es sobre `tmdb_state`.
  Pendiente para cuando haga falta de verdad en la ficha de película (F5).
- **`queries.rs` no depende de `catalog::normalize`.** `upsert_movie`/`upsert_series`/
  `upsert_channel` reciben un `TitleFields` con los valores ya calculados; quien
  normaliza es `catalog::sync`. Mantiene el SQL en `db`, la lógica de negocio en
  `catalog`.
- **Recalcular `match_key` orfanaba filas de F2.** Al cambiar de placeholder a
  normalización real, el `ON CONFLICT(match_key)` de una resincronización ya no
  encuentra la fila vieja: se crea una nueva y la vieja se queda sin `movie_sources`/
  `series_sources`. Se añadió `delete_orphaned_movies_and_series`, llamada al final de
  cada `sync::run`.
- **TMDB corre automáticamente tras cada sincronización, y también a mano** desde
  Ajustes ("Buscar en TMDB"), para cuando la clave se añade después de haber
  sincronizado ya — si no, esos pendientes se quedarían así para siempre.
- **Estado `error` se reintenta junto con `pending`.** Representa un fallo transitorio
  (red, 429 agotado), no un "TMDB no tiene esto" definitivo como `not_found`.
- **Concurrencia de TMDB**: un `tokio::spawn` por película/serie pendiente (pueden ser
  miles), con un `tokio::sync::Semaphore` de 5 dentro de `TmdbClient` limitando las
  peticiones HTTP reales en vuelo, y un único `Connection` compartido tras un
  `tokio::sync::Mutex` para la caché y la escritura del resultado (bloqueos breves,
  de una sola fila). Se prefirió a abrir varias conexiones SQLite concurrentes o a
  añadir la dependencia `futures` solo para `buffer_unordered`.
- **Ninguna dependencia nueva.** La similitud de texto es Levenshtein normalizado
  escrito a mano.

**Deuda técnica pendiente.**
- Heredada de F1/F0 (persistencia de URL M3U, CORS permisivo, icono provisional).
- Sin botón de cancelar el pase de TMDB desde la interfaz (el motor acepta un `cancel`,
  falta exponerlo).
- El 80% de aciertos del criterio de aceptación no se ha podido verificar de extremo a
  extremo: hace falta una clave de TMDB real, que no se ha pedido al usuario. Queda para
  cuando el usuario la añada desde Ajustes.

---

## 2026-08-31 — F4 API local y tipos

**Qué se ha construido.** `server/api.rs` expone el catálogo por HTTP en axum, todavía
solo en 127.0.0.1: `/api/home`, `/api/live/categories`, `/api/live/channels`,
`/api/movies` (+ `/:id`), `/api/series` (+ `/:id`, con `series_detail()` bajo demanda si
`detail_synced_at` es nulo o tiene más de 7 días), `/api/search`, `/api/playback/progress`,
`/api/favorites` (GET/POST/DELETE). `GET /img/:size/:image_id` (de F3) sirve portadas y
fondos. `db/reads.rs` es la nueva capa de lectura (paginación por cursor, filtros de
género vía `json_each` sobre la columna `genres`, búsqueda por FTS5). 27 tipos ts-rs
nuevos exportados. 22 tests nuevos (12 en `reads.rs`, 10 en `server::api`).

**Decisiones tomadas.**
- **`search_index` (FTS5) se reconstruye entera al final de cada sincronización, no con
  triggers por fila.** Con decenas de miles de upserts por sincronización, un trigger de
  borrado por fila sobre una tabla FTS5 sin índice en `item_id` habría sido O(n²) en la
  práctica. Reconstruirla es un único escaneo por tabla (`channels`, `movies`, `series`).
- **La paginación por cursor usa `id` como desempate único, incluso con `sort` distinto
  de "más recientes".** Hacer keyset pagination exacto para cada combinación de
  orden/filtro no compensaba el tiempo en esta fase, sin una interfaz real que lo
  ejerciera todavía (llega en F5). Documentado como límite conocido, no un descuido.
- **`db/reads.rs` separado de `db/queries.rs`**: lectura (API HTTP) frente a escritura
  (sincronizador). `queries.rs` sigue sin depender de `catalog`; `reads.rs` tampoco.
- **Conexión nueva por petición HTTP, vía `spawn_blocking`**, sin pool de conexiones
  (`r2d2`/`bb8`): SQLite + WAL soporta lectores concurrentes de sobra para el tráfico de
  una app de escritorio de un único usuario, y añadir un pool no estaba justificado.
  `open_readonly_at` evita repetir la comprobación de migraciones en cada petición.
- **`build_source_for_provider` se extrajo de `sync_provider`** a una función
  compartida en `commands.rs`, reutilizada por la ficha de serie de F4 para reconstruir
  el `Source` y llamar a `series_detail()` bajo demanda.
- **Genero de películas/series filtra sobre el JSON crudo de TMDB (`json_each`)**, sin
  tabla de nombres de género: el criterio de aceptación es sobre tiempos de respuesta y
  coherencia, no sobre mostrar el nombre "Acción" en vez del id `28` — eso es trabajo de
  interfaz de F5.

**Verificación real (no queda en el repo).** Con el proveedor Xtream real del usuario
(el mismo de F2/F3), se sincronizó a la base de datos real de la aplicación (no una de
prueba): 615 canales, ~14.000 películas y ~7.900 series tras la normalización real de
F3. Contra ese catálogo completo: `/api/movies`, `/api/series`, `/api/live/channels`,
`/api/search`, `/api/home` respondieron todos entre 15 y 40 ms — muy por debajo del
límite de 100 ms del criterio de aceptación. `/api/series/:id` con recarga bajo demanda
(primera visita, sin caché) tardó ~3 s por la llamada real a `get_series_info` del
proveedor — esperado y documentado, no es una regresión del límite de 100 ms de las
rutas de solo lectura.

**Deuda técnica pendiente.**
- Heredada de fases anteriores (ver entradas previas).
- Sin nombres de género legibles (solo ids numéricos de TMDB) — F5.
- Paginación por cursor no es exacta combinada con `sort` != "recientes" (ver arriba).
- El proveedor de prueba en la base de datos real quedó con el nombre "P" (un
  placeholder de test): cosmético, se puede renombrar desde Ajustes cuando exista la
  opción de editar (no construida todavía).

---

## 2026-08-31 — F5 Cascarón de interfaz

**Aviso importante sobre esta fase.** `PROMPTS.md` dice explícitamente "Fase 5 es de
diseño: itera pantalla a pantalla, no aceptes la primera versión" — pero esta fase se
hizo en modo autónomo, sin que el usuario estuviera delante para revisarla ni pedir
cambios. Lo que hay aquí es una primera versión funcional y razonablemente cuidada, no
el resultado de esa iteración. Verificado: `tsc --noEmit` limpio, `npm run tauri dev`
arranca sin errores de compilación. **No verificado: cómo se ve ni se siente de verdad**
— no hay forma de comprobar visualmente una interfaz desde este entorno. Cuando el
usuario la abra, hay que tratarlo como el punto de partida de la iteración que pide
`PROMPTS.md`, no como una fase cerrada de verdad en el sentido que ahí se describe.

**Qué se ha construido.** Las 8 pantallas: Inicio (filas de continuar viendo/recientes),
Directo (lista virtualizada con filtro de categoría), Películas y Series (rejillas
virtualizadas con scroll infinito por cursor), Detalle de película y de serie (fondo a
sangre con degradado, sinopsis, metadatos; la de serie con selector de temporada),
Buscar (instantáneo, rebote de 150 ms) y Ajustes (de F2/F3, reencajada en el nuevo
layout). Barra lateral colapsable con iconos SVG propios (sin librería de iconos).
`components/VirtualPosterGrid.tsx` implementa la rejilla con TanStack Virtual: columnas
recalculadas por `ResizeObserver`, filas virtualizadas, scroll infinito, y navegación por
flechas con reintento de foco por `requestAnimationFrame` (la tarjeta destino puede no
existir aún en el DOM al pulsar la flecha, por estar virtualizada). `Row.tsx` hace lo
mismo en horizontal para las filas de Inicio. Estados de carga con esqueletos
(`animate-pulse`), nunca un spinner centrado. Escape vuelve atrás en las fichas de
detalle.

**Decisiones tomadas.**
- **`@tanstack/react-virtual` añadido** (dependencia nueva, justificada: pedida
  explícitamente por `docs/CLAUDE.md`/`PROMPTS.md` para rejillas y listas grandes).
- **Iconos SVG propios en vez de una librería** (`lucide-react`, etc.): media docena de
  trazos simples no justifican una dependencia nueva.
- **`GET /api/*` se llama por `fetch` desde React, no por comandos Tauri.** Con F4 ya
  construido, el catálogo pasa a consumirse por HTTP tal como describe
  `docs/ARCHITECTURE.md`; los comandos Tauri de F2/F3 (proveedores, TMDB) se quedan como
  están hasta que F4 los cubra también, si es que llega a hacerlo.
- **El logo de canal se sigue sirviendo desde la URL directa del proveedor**, sin
  proxy ni caché en disco (a diferencia de portadas/fondos de TMDB, que sí pasan por
  `/img/*`). Ya se redactan credenciales si las hubiera (fix de F5 en `db/reads.rs`), pero
  no hay caché de logos: se consideró fuera de alcance de esta fase.
- **Botones de "Reproducir" y de canales están deshabilitados/sin acción**, con un
  `title` explicando que llegan en F6/F7: mejor un botón visiblemente inerte que uno que
  aparente funcionar y no haga nada.

**Un bug real de R5 encontrado revisando esta fase, no relacionado con el frontend en
sí:** `GET /api/live/channels` (F4) devolvía `logo_url` tal cual venía del proveedor,
contradiciendo el propio comentario del fichero ("ninguna respuesta lleva... URLs del
proveedor"). Corregido pasando `logo_url` por `redact()` en `db/reads.rs` antes de
salir de la capa de datos, con un test que confirma que credenciales embebidas en un
logo (`/live/usuario/contraseña/logo.png`) no llegan a la respuesta HTTP.

**Deuda técnica pendiente.**
- Toda la de fases anteriores.
- **La propia fase, en el sentido de `PROMPTS.md`**: falta la iteración con el usuario
  screen a screen que el plan de fases da por hecha.
- Caché/proxy de logos de canal (ver arriba).
- Navegación por teclado cubierta a nivel de fila/rejilla (flechas + Enter + Escape en
  fichas); no hay todavía un sistema de "TV navigation" más completo (focus trapping en
  modales, indicadores de dirección) — razonable para v1 de escritorio con ratón y
  teclado, insuficiente si se apunta a mando a distancia sin más ajustes.

---

## 2026-08-30 — F1 Capa de fuentes

**Qué se ha construido.** Trait `Source` (`#[async_trait]`) en `sources/mod.rs` con
`XtreamSource` y `M3uSource` completos. Tipos compartidos en `sources/model.rs`,
`SourceError` (`thiserror`) en `sources/error.rs`, deserializadores tolerantes a
número/cadena/nulo/ausente en `sources/serde_helpers.rs`. `XtreamSource` cubre cuenta,
categorías, canales, películas, series, `get_series_info`, User-Agent configurable y
construcción de URL de stream en sus dos variantes (con y sin `/live/`). `M3uSource`
parsea línea a línea, extrae el nombre visible tras la última coma, clasifica por ruta
con heurística de `group-title` como respaldo, agrupa episodios por patrón
(`S01E02`/`1x02`/`T1 E2`/`Temporada X Capitulo Y`) y expone `detect_embedded_xtream_credentials`
para URLs `get.php`. 20 fixtures anónimos en `tests/fixtures/` (8 respuestas Xtream + una
lista M3U de 50 entradas con los casos raros). 41 tests, todos en verde.

**Decisiones tomadas.**
- **El reintento sin `/live/` no vive en F1.** `docs/XTREAM-API.md` describe probar la
  URL, fallar con 404 y reintentar sin el segmento — pero eso implica abrir una conexión
  real al stream, y R1 prohíbe explícitamente abrir un stream para comprobar si
  funciona. `XtreamSource` expone `stream_url_variant()` con ambas formas; probarlas de
  verdad y recordar cuál funciona queda para el proxy de `/stream/:kind/:id` de F6, que
  ya abre el stream a través de StreamGuard de todas formas.
- **`remote_id` de M3U es un hash opaco, no una redacción de la URL.** `util::redact()`
  solo reconoce la forma `/live|movie|series/usuario/contraseña/id` de Xtream. Un M3U
  puede traer credenciales bajo cualquier segmento de ruta (`/vod/`, lo que sea);
  ampliar `redact()` a cualquier ruta de 3+ segmentos habría sido peligroso (destroza
  URLs de imágenes con esa forma). Un hash del contenido no puede filtrar un substring
  del original, sea cual sea la forma de la URL. Se encontró probando la fixture propia
  de test, con rutas `/vod/`/`/x/` a propósito para forzar la heurística de
  `group-title`.
- **Wire structs máximamente tolerantes, filtrado explícito en la conversión.** Cada
  tipo `*Wire` de Xtream tiene todos los campos opcionales (incluidos los ids); la
  conversión a `Raw*` descarta la entrada individual si falta un id imprescindible, en
  vez de fallar el listado completo por una fila mal formada.
- **Categorías sintéticas para M3U.** M3U no tiene endpoint de categorías; se derivan de
  los valores distintos de `group-title` vistos por tipo, con un slug interno como
  `remote_id`. `RawSeries` no lleva categoría (a diferencia de `RawChannel`/`RawMovie`):
  `series_sources` en `docs/DATA-MODEL.md` tampoco tiene esa columna.
- **Testeo de Xtream con un mock axum, no una librería de mocking HTTP.** axum ya es
  dependencia de producción; un router mínimo en el propio test sirve los fixtures y
  permite comprobar cabeceras (`User-Agent`) sin añadir ninguna dependencia nueva solo
  para tests.
- **`rust-version` corregido a 1.82.** Clippy avisó de que `LazyLock` (usado en
  `m3u.rs`) e `Option::is_none_or` (usado en `redact.rs` desde F0) piden 1.80 y 1.82
  respectivamente; el `1.77` de F0 era una cifra puesta sin comprobar.

**Deuda técnica pendiente.**
- **Persistencia de la URL de reproducción de M3U entre reinicios.** A diferencia de
  Xtream (que reconstruye la URL siempre desde usuario+contraseña+id), M3U no tiene esa
  estructura: la URL real solo vive en memoria dentro de la instancia de `M3uSource` que
  hizo el parseo. `docs/DATA-MODEL.md` no tiene columna pensada para esto. F2 tiene que
  decidir cómo resolver una URL de M3U después de un reinicio de la app (¿columna nueva?
  ¿re-parsear la lista al vuelo?) — no lo he decidido por mi cuenta.
- CORS permisivo y icono provisional, heredados de F0 (ver entrada anterior).
- `sources` sigue con `#[allow(dead_code)]` en `main.rs`: sin consumidor en producción
  hasta que `catalog::sync` (F2) llame de verdad al trait `Source`.

---

## 2026-08-31 — F6 Reproductor

**Aviso importante sobre esta fase.** `PROMPTS.md` la marca como "la parte más
delicada del proyecto" y pide hacerla por partes, enseñando cada una funcionando — pero
se ha construido en modo autónomo, sin el usuario delante. He podido verificar de
verdad, con ffmpeg/ffprobe reales instalados en esta máquina (no simulados) y sin tocar
ninguna credencial real: que la sonda de códecs decide el nivel correcto, que el remux
produce fMP4 válido byte a byte, que el proxy nivel 1 sirve Range de verdad, y que 20
aperturas de stream seguidas no dejan procesos ffmpeg huérfanos (los cuatro, contra
servidores HTTP locales sirviendo ficheros generados por el propio ffmpeg). **Lo que no
he podido verificar es lo único que de verdad importa de un reproductor: que un canal se
vea y se oiga bien, que el audio vaya sincronizado, que el zapping se sienta fluido.**
Eso solo lo puede confirmar quien tenga ojos y oídos delante de la ventana de la app.

**Qué se ha construido.**
- **Parte A — `media/probe.rs`.** `probe_url()` lanza `ffprobe` (vía `tokio::process`,
  `kill_on_drop`, timeout de 10 s) y extrae códec de vídeo, de audio y contenedor.
  `decide_tier()` es una función pura (muy testeada: 12 casos) que decide 1/2/3 según la
  tabla de `docs/ARCHITECTURE.md`. `stream_cache_key()` genera la clave de `probe_cache`
  con un hash (nunca la URL en claro, regla R5).
- **Parte B — `GET /stream/:kind/:id` en `server/stream.rs`.**
  - Resuelve `provider_id`+`remote_id` desde la base (`db/reads.rs`:
    `resolve_channel_stream`/`resolve_movie_stream`/`resolve_episode_stream`, nuevas),
    reconstruye el `Source` con `commands::build_source_for_provider` (ya existía, de
    F4) y pide `stream_url()`.
  - Nivel 1: proxy transparente con soporte de Range (cabeceras reenviadas tal cual).
  - Nivel 2: `media/ffmpeg.rs::spawn_remux` con los flags exactos del documento; audio a
    AAC solo si es AC3/E-AC3/DTS; `-ss` antes de `-i` cuando el cliente pide
    `?start=<segundos>` (así implementa la búsqueda en VOD remuxeado).
  - `util::stream_guard::StreamGuard` ahora de verdad mata y espera (`.wait().await`,
    no solo la señal) el proceso ffmpeg de la sesión anterior antes de devolver el
    permiso siguiente. Un `PermitBoundBody` envuelve el cuerpo de la respuesta HTTP para
    que, se acabe el stream solo o corte el cliente la conexión, se libere el permiso y
    se mate el proceso — un único `Drop` cubre los dos casos.
  - `GET /api/playback/tier/:kind/:id`: lee `probe_cache` sin abrir ninguna conexión
    (resolver la URL es cálculo local), para el panel de estadísticas del reproductor.
  - `GET /api/playback/progress?item_kind=&item_id=`: progreso guardado de un ítem
    concreto (existía el POST desde F4; faltaba poder leer uno solo para "reanudar").
- **Parte C — `features/player/PlayerScreen.tsx`.** Un único componente para directo,
  película y episodio (`/player/:kind/:id`), fuera del `AppLayout` (pantalla completa,
  sin barra lateral). Controles superpuestos que se ocultan a los 3 s de inactividad,
  barra de progreso, volumen, pantalla completa, guardado de posición cada 10 s y al
  desmontar, reanudar con aviso ("Reanudando desde MM:SS"), siguiente episodio con
  cuenta atrás cancelable, zapping en directo con flechas (dentro de la categoría del
  canal), panel de estadísticas tras Ctrl+Shift+D (nivel, códecs, resolución, tiempo al
  primer fotograma, buffer).

**Decisiones tomadas.**
- **El reintento sin `/live/` (pendiente desde F1) se resuelve aquí.** `Source` gana un
  método `stream_url_fallback()` con implementación por defecto `None`; `XtreamSource`
  lo sobreescribe para el directo. `resolve_tier_and_url()` prueba la URL principal y,
  si `ffprobe` falla, reintenta con la alternativa antes de rendirse — con las dos
  ocurriendo secuencialmente (nunca en paralelo), respeta el límite de conexiones.
- **Sin sondear más de una vez por ítem, ni fuera de la sesión que ya se iba a abrir.**
  R1 prohíbe el prefetch, pero sondear la URL que el usuario ya pidió reproducir no lo
  es: es la misma conexión, no una especulativa. Se cachea en `probe_cache` por hash de
  URL: si el hash de la URL principal falla y hace falta la alternativa, se cachea bajo
  el hash de la que de verdad funcionó — la próxima vez que la principal falle, se
  vuelve a sondear (degradación honesta: nunca se rompe en silencio, solo re-sondea algo
  más para paneles antiguos).
- **Sin `hls.js` en esta fase**, pese a que `PROMPTS.md`/`docs/ARCHITECTURE.md` lo
  mencionan para el nivel 1. El nivel 1 real hoy (MP4 progresivo con H.264/AAC) y el
  nivel 2 (fMP4 remuxeado) se sirven los dos igual de bien con un `<video src>` nativo,
  sin necesitar MSE. HLS de verdad (`.m3u8`) sí está reconocido y proxied de forma
  transparente en el backend (`handle_stream`), pero **sin reescribir las URIs internas
  del manifiesto** y **sin integrar `hls.js` en el cliente todavía** — no se ha podido
  verificar contra ningún proveedor real que sirva HLS (ni Xtream ni M3U del usuario lo
  hacen). Añadir `hls.js` cuando aparezca tráfico real en ese formato es deuda explícita,
  no un olvido.
- **Cómo decide el reproductor si puede buscar de forma nativa.** En vez de exponer el
  nivel al cliente antes de reproducir (que obligaría a sondear dos veces, o a arriesgar
  una conexión especulativa concurrente con la que ya estuviera activa), el `<video>`
  mira `Number.isFinite(video.duration)` tras `loadedmetadata`: finito = nivel 1
  byte-seekable de verdad (`video.currentTime =` funciona solo), infinito = nivel 2, y
  buscar significa pedir un stream nuevo con `?start=`. Al primer resumen (`resume`) se
  manda `?start=<posición>` siempre por si acaso es nivel 2; si resulta ser nivel 1, se
  corrige con una búsqueda nativa real en cuanto se conoce la duración. Ningún sondeo
  adicional, ninguna conexión de más.
- **`StreamGuard` pasa a poseer el `Child` de ffmpeg**, no solo el flag booleano de F2.
  `acquire()` mata y espera al de la sesión anterior; `attach_process`/`release_process`
  lo registran y lo liberan. Se consideró tener el proceso solo en el cuerpo de la
  respuesta HTTP (más simple), pero entonces "cerrar el anterior antes de abrir el
  siguiente" no tendría un punto único y determinista si el cliente tardaba en notar la
  desconexión — con el proceso en el guard, `acquire()` es ese punto, siempre.
- **Los binarios de ffmpeg/ffprobe se buscan en el `PATH`** (`media::resolve_binary`),
  no como sidecar de Tauri todavía. Empaquetarlos en `src-tauri/binaries/` con
  `externalBin` es trabajo de F10 (`docs/PROMPTS.md` los lista ahí explícitamente); hacerlo
  ahora habría significado inventar un binario que no existe en este repo. Si faltan, el
  endpoint responde 503 en vez de entrar en pánico.
- **Dependencias nuevas**: `tokio` (features `process`/`time`/`io-util`/`fs`),
  `tokio-util` (`io`, para `ReaderStream`), `futures-core`/`futures-util` (combinadores
  de `Stream`), `bytes`, `reqwest` feature `stream`, `tower-http` feature `fs` (solo en
  tests, para servir un fichero real con Range vía `ServeFile`). Todas justificadas por
  necesitar streaming de bytes real entre procesos/HTTP; ninguna sustituible por código
  propio sin reinventar combinadores de `Stream` estándar.

**Verificación real hecha (sin credenciales, con ffmpeg/ffprobe instalados en esta
máquina).**
- `probe.rs`: sonda un MP4 h264/aac generado con ffmpeg de verdad (no solo JSON escrito
  a mano) y confirma nivel 1.
- `ffmpeg.rs`: remux real de un TS servido por HTTP a fMP4 por stdout; confirma cajas
  `ftyp` válidas en los primeros bytes.
- `server/stream.rs`: dos pruebas de extremo a extremo por HTTP (axum `oneshot`) contra
  un proveedor Xtream simulado (`tower_http::ServeFile` sirviendo el fichero real en la
  ruta exacta `/movie/user/pass/1.mp4` / `/live/user/pass/1.ts`) — nivel 1 con Range y
  nivel 2 con remux, las dos confirmando el `play_tier` guardado en `probe_cache`.
- Verificación manual (marcada `#[ignore]`, se queda en el repo para repetirla a mano
  tras tocar `StreamGuard`): 20 apertures seguidas de `/stream/channel/:id` sin leer el
  cuerpo (simula cambiar de canal antes de que cargue) — `tasklist` confirma cero
  `ffmpeg.exe` huérfanos al terminar.
- `tsc --noEmit` y `vite build` limpios; `npm run tauri dev` arranca sin errores en el
  terminal.

**Bug real encontrado por el usuario, en el primer minuto de probarlo.** Abrió un canal
y una película de su proveedor real: las dos se veían y se oían bien, pero la película
no se podía avanzar — la barra de progreso se comportaba "como si estuviera en directo".
Causa: la única fuente de duración era `runtime_min` de TMDB (dato editorial, a menudo
ausente), y sin duración la barra ni se pintaba. La duración real del fichero ya la sabía
`ffprobe` (`format.duration`) pero `ProbeResult` no la recogía. Arreglado en las cuatro
capas: `probe.rs` la extrae, `probe_cache` gana una columna (`duration_s`, migración
`002_duracion_probe_cache.sql` — nunca se edita la 001 ya publicada), `GET
/api/playback/tier` la expone, y `PlayerScreen` la usa como fuente principal de
duración (con `runtime_min`/`duration_s` del episodio como respaldo si todavía no se ha
sondeado nunca). Se descarta explícitamente en directo: la "duración" que ve `ffprobe`
ahí es la del trozo que llegó a analizar, no la del canal, que no tiene fin. Confirmado
con test end-to-end contra un MP4 real (duración esperada ~2 s) y contra un TS en
directo (confirma que ahí se guarda `NULL`). La caché de sondeo del proveedor real del
usuario se limpió a mano una vez (`DELETE FROM probe_cache`) para que el siguiente play
sondee de nuevo con el código corregido — necesario porque la caché no expira y ya
tenía una entrada sin duración de antes del fix.

**Segunda vuelta del mismo bug: la primera corrección no bastaba.** Tras el arreglo de
arriba, el usuario probó de nuevo y la barra seguía mal — "0:07 / 0:01" con la película
ya por el segundo 7. La causa real no era la ausencia de duración, sino de dónde se
leía: el código priorizaba `video.duration` (la del propio elemento `<video>`) sobre la
de `ffprobe`, y para un fMP4 fragmentado servido por `<video src>` sin MSE, Chromium
puede reportar una duración finita pero **provisional y equivocada** mientras llegan
los primeros fragmentos — no es un dato fiable para distinguir nivel 1 de nivel 2, ni
para mostrar la duración total. Corregido: `nativeSeekable` ahora se decide por el
nivel real (`tierQuery.data.tier === 1`, ya cacheado por la sonda), no por
`Number.isFinite(video.duration)`; `displayDuration` usa siempre `ffprobe` (vía
`tierQuery`) o los metadatos editoriales, nunca `video.duration`; y la corrección de
posición al reanudar en nivel 1 usa `video.seekable.length > 0` (lo que el navegador
demuestra de verdad sobre soporte de Range) en vez de la duración. Aprendido: para un
`<video>` alimentado con un stream generado por el propio proxy, cualquier propiedad que
el navegador infiera progresivamente (`duration` incluida) es sospechosa hasta que se
demuestre estable — mejor apoyarse en lo que el núcleo ya sabe con certeza.

**Tercera vuelta: la barra desapareció del todo.** Comprobado contra la base de datos
real del usuario y contra el propio `GET /api/playback/tier` en caliente: el núcleo
tenía el dato correcto (`tier: 2, duration_s: 8834.08`, ~2h27, coherente con una
película). El fallo estaba en el cliente: `tierQuery` se pide en paralelo al montar,
justo cuando el `/stream/*` de esa misma carga puede no haber terminado de sondear
todavía, así que la primera respuesta puede llegar con la caché aún vacía; el único
reintento que había (`onLoadedMetadata`) asumía un orden de eventos no garantizado.
Arreglado con `refetchInterval` condicional: reintenta cada segundo mientras no se
conozca el nivel, para en cuanto se conoce. Además, ahora hay botones «10s/10s» siempre
visibles en vídeo/episodio (no solo la barra numérica): si por lo que sea la duración
tarda en saberse, buscar hacia detrás/delante nunca depende de que se haya pintado la
barra.

**Cuarta vuelta: la causa real, encontrada con el panel de estadísticas como
instrumento.** Las tres correcciones anteriores eran necesarias pero ninguna era la
causa raíz. Se añadieron filas de diagnóstico al panel (Ctrl+Shift+D) y el usuario mandó
una captura: "Nivel: desconocido" incluso con la película ya por el minuto 1 — es decir,
`GET /api/playback/tier` nunca llegaba a resolverse en el cliente, pese a que llamarlo
a mano con `curl` contra el mismo puerto devolvía el dato correcto al instante. La
diferencia entre `curl` y el `fetch()` del WebView es CORS, y ahí estaba: el
`.layer(CorsLayer)` de F0 se aplicaba dentro de `routes::build_router()`, **antes** de
que F6 fusionara `stream::stream_router` (que añade `/stream/*` y
`/api/playback/tier/*`) en `server::serve()`. Un `.layer()` en axum solo cubre las rutas
que ya existen en el router en el momento de llamarlo, no las que se fusionan después:
esas dos rutas nunca llevaron cabecera CORS. `<video src>` no lo notó porque cargar un
recurso de vídeo no pasa por CORS (solo `fetch`/`XHR` lo hacen), así que la reproducción
en sí siempre funcionó — únicamente `fetch()` a `/api/playback/tier` fallaba, en
silencio, cada vez, para siempre. Las tres correcciones anteriores estaban resolviendo
problemas reales que habrían aparecido de todas formas en cuanto esta petición
funcionara; sin este arreglo, ninguna podía demostrarse.

Arreglado moviendo el `CorsLayer` a `server::full_router()`, aplicado sobre el router ya
fusionado por completo (catálogo + salud/imágenes + streaming), con un test de
regresión que pide una ruta de F6 y comprueba la cabecera — para que este error de nuevo
no pueda colarse en silencio. Lección para las fases futuras que añadan rutas: el CORS
se aplica una vez, al final, sobre el router completo, nunca sobre una mitad antes de
fusionar.

**Confirmado por el usuario tras el arreglo de CORS: la barra de progreso y la búsqueda
en VOD ya funcionan.**

**Deuda técnica pendiente.**
- Toda la de fases anteriores.
- Series, zapping y reanudar no se han probado todavía con el proveedor real del
  usuario (audio/vídeo en directo y en película sí están confirmados, con la búsqueda ya
  corregida).
- HLS: proxy transparente sin reescritura de manifiesto, sin `hls.js` en el cliente (ver
  decisión arriba).
- **Solo una fuente/calidad por película en el reproductor** (la primera de
  `movie_sources`, sin selector); `?source=<id>` ya lo soporta el backend, falta la UI.
- **Pistas de audio y subtítulos no seleccionables.** El remux solo copia una pista de
  audio (la que ffmpeg mapea por defecto) y ninguna de subtítulos: falta lógica de
  `-map` para exponer varias, y la interfaz para elegir. `docs/PROMPTS.md` pide el
  control en la interfaz; no hay nada que controlar todavía en el flujo de datos.
- **Zapping limitado a 200 canales de la categoría actual** (o los primeros 200 sin
  categoría): razonable para listas reales, pero no es "toda la lista" en categorías
  enormes.
- **Nivel 3 (mpv) no implementado**: cualquier ítem que decida nivel 3 responde 501.
  Es trabajo de F9 según el plan de fases.
- **Catch-up/timeshift no implementado**: F9.
- **Sin mensajes de error en español para el usuario final** más allá de un texto
  genérico ("no se pudo reproducir..."): F9 pide errores claros por causa (cuenta
  caducada, límite de conexiones, sin conexión).
- `ffmpeg`/`ffprobe` se buscan en el `PATH` (ver decisión arriba): el sidecar de F10
  sustituye esto por una ruta empaquetada.

---

## 2026-08-31 — F7 EPG y directo

**Construida de manera autónoma**, siguiendo la instrucción de continuar sin
supervisión mientras el usuario no estuviera delante. A diferencia de F6, no ha hecho
falta ninguna corrección posterior: se verificó contra el proveedor real del usuario
(sin guardar ninguna credencial) antes de darla por cerrada.

**Qué se ha construido.**
- **`catalog/epg.rs`**: `download_xmltv()` descarga por trozos a disco (nunca en
  memoria); `parse_xmltv()` parsea con `quick-xml` evento a evento, descartando fuera
  del propio bucle (antes de construir nada) los programas anteriores a ayer o
  posteriores a siete días. Descomprime gzip sniffeando los bytes mágicos (`1f 8b`) en
  vez de fiarse de `Content-Encoding` o de la extensión del fichero: muchos paneles
  sirven el `.gz` sin declarar la cabecera. Fechas XMLTV (`YYYYMMDDHHMMSS ±ZZZZ`)
  parseadas a mano con el algoritmo de Howard Hinnant (aritmética de enteros,
  dominio público) para no añadir una dependencia de fechas solo para un formato fijo.
  `run_epg_sync()` orquesta descarga + parseo + reemplazo completo de la guía (como
  `search_index`: más simple que reconciliar, y XMLTV no da un id de programa estable
  con el que hacer upsert) + informe de cuántos canales quedan sin EPG.
- Enganchado como paso 8 tras cada sincronización de catálogo (`commands::spawn_sync`),
  en segundo plano, igual que TMDB. Si la fuente no ofrece XMLTV (M3U sin
  `x-tvg-url`), no hace nada — no es un error.
- **`GET /api/epg/now?channel_ids=1,2,3`**: programa actual y siguiente por canal (id
  interno, nunca el id de XMLTV, que la API nunca expone). **`GET
  /api/epg/grid?channel_ids=&from=&to=`**: todos los programas que se solapan con la
  ventana pedida, para la vista en rejilla.
- **Frontend**: `LiveScreen` muestra el programa actual con una barra de progreso fina
  bajo cada canal (refrescado cada minuto). Pantalla nueva **Guía** (`/guide`, entrada
  de navegación propia): rejilla canal×tiempo de 4 horas, flechas izquierda/derecha
  desplazan la ventana temporal, arriba/abajo mueven el foco entre canales, Enter abre
  el directo de la fila (no el programa concreto: catch-up es F9). `PlayerScreen`
  enseña una superposición con "qué están dando" (actual y siguiente) al entrar a un
  canal o hacer zapping, que se retira sola a los 5 s.

**Decisiones tomadas.**
- **Sin `chrono` ni ninguna librería de fechas nueva.** XMLTV usa un único formato fijo;
  parsearlo a mano con aritmética de enteros (mismo principio que ya se usaba en
  `redact()`/`normalize()`: preferir código propio pequeño y bien testeado a una
  dependencia para un problema acotado) evita añadir peso solo para esto.
- **`quick-xml` sí es una dependencia nueva justificada**: parsear XML a mano
  evento a evento sin un parser de verdad habría sido reinventar un tokenizador SAX
  entero, con muchas más formas de equivocarse que aritmética de fechas.
- **La guía se reemplaza entera, no se reconcilia fila a fila** (ver arriba): coherente
  con cómo ya se trata `search_index` en `catalog::sync`.
- **`GET /api/epg/now` hace una consulta por canal, no una sola con `IN`.** Para la
  lista de "ahora" (decenas de filas visibles, no miles) es más simple y suficientemente
  rápido; `GET /api/epg/grid` sí usa un único `IN` porque ahí el número de programas por
  ventana puede ser mucho mayor.
- **La rejilla de guía pagina a 60 canales por categoría**, sin virtualizar ni el eje de
  canales ni el de tiempo: razonable para una categoría típica, documentado como límite
  conocido más que como virtud.

**Verificación real hecha (proveedor del usuario, sin guardar ninguna credencial: test
temporal con variables de entorno, ejecutado una vez y borrado antes de este commit,
confirmado con un `grep` del repo).** Su XMLTV real: 5,7 MB descargados en 3,8 s,
11.369 programas parseados en 562 ms. Muy por debajo del criterio de aceptación (30 MB
en menos de 30 s) y coherente por escala. La guía en la UI todavía no se ha visto con
datos reales del usuario: la sincronización de su proveedor se hizo antes de que F7
existiera, así que hace falta un resincronizado (botón "Sincronizar" de Ajustes, F2)
para que su guía real llegue a `channels`/`epg_programmes` y se pueda ver en Directo y
en la Guía.

**Confirmado por el usuario tras resincronizar: Directo, Guía y el reproductor
funcionan perfectamente con su programación real.**

**Deuda técnica pendiente.**
- Toda la de fases anteriores.
- Sin caché de refresco periódico de EPG en segundo plano: se actualiza solo al
  resincronizar el proveedor a mano, no cada X horas por su cuenta.
- Guía sin virtualizar (ver decisión arriba) y limitada a 60 canales por categoría.
- Catch-up/timeshift desde una celda de la guía: F9.

---

## 2026-08-31 — F8 Búsqueda, favoritos y continuar viendo

**Construida de manera autónoma.** La búsqueda instantánea (FTS5, rebote 150 ms,
agrupada por tipo) ya existía desde F5, adelantada a esta fase; aquí se ha completado lo
que quedaba: navegación real de resultados de canal (antes no hacía nada), favoritos de
verdad, "continuar viendo" con datos reales en vez de solo IDs, "siguiente episodio" y
control parental con PIN.

**Qué se ha construido.**
- **Favoritos**: `useFavorites()` (una lista, actualización optimista) y un
  `HeartIcon` conmutable en la esquina de cada tarjeta de `VirtualPosterGrid` (con
  `favoriteKind="movie"|"series"`) y en la ficha de película/serie, junto al botón de
  reproducir. El backend ya existía de F4 (`GET/POST/DELETE /api/favorites`); no ha
  hecho falta tocarlo.
- **Continuar viendo con datos reales**: `GET /api/home` antes solo daba
  `item_kind`/`item_id` en crudo, y el frontend no podía pintar nada útil con eso.
  `reads::list_continue_watching` ahora enriquece cada fila con título y portada de la
  tabla que corresponda (película, o serie+temporada+episodio para un episodio) — una
  consulta por ítem, no una `UNION`: son como mucho unas pocas decenas de filas.
  **Directo se excluye a propósito**: un canal no se "continúa", y sin este filtro cada
  canal visto por zapping aparecía para siempre con 0% visto — encontrado probando
  contra los datos reales del usuario, no en el diseño de la tabla.
- **Siguiente episodio pendiente por serie**: `reads::list_next_episodes` mira, por cada
  serie con algún episodio en `playback_state`, cuál fue el último reproducido y calcula
  el inmediatamente siguiente (misma temporada, número mayor; o el primero de la
  siguiente temporada) — misma lógica que `findNextEpisode` de `PlayerScreen.tsx`, ahora
  también en el servidor. Nueva fila "Siguiente episodio" en Inicio.
- **Control parental con PIN**: `util::parental` (hash SHA-256, nunca el PIN en claro;
  `ParentalGuard` en memoria — el desbloqueo nunca se persiste, se pierde al reiniciar
  la app a propósito). `GET /api/parental/status`, `POST /api/parental/{pin,unlock,lock}`.
  Mientras no se desbloquea, `list_categories`/`list_channels_page`/`list_movies_page`
  excluyen lo marcado `is_adult` — series no tiene columna de categoría, así que no hay
  nada que ocultar ahí a ese nivel. Sección nueva en Ajustes para establecer/cambiar/
  quitar el PIN y desbloquear.
- **Quién marca una categoría como adulta**: no hay endpoint ni UI para hacerlo a mano
  todavía. `queries::upsert_categories` aplica una heurística por nombre (`XXX`,
  `ADULT`, `+18`, `18+`, `EROTIC`...) en cada sincronización — vista en paneles Xtream
  reales, no inventada. Renombrar una categoría la reclasifica sola en la siguiente
  sincronización.

**Decisiones tomadas.**
- **Sin marcado manual de categorías adultas todavía**: la heurística por nombre cubre
  el caso real típico sin necesitar una pantalla nueva; corregirla a mano si se
  equivoca es deuda pendiente, no un olvido.
- **`sha2` es una dependencia nueva justificada**: hashear un PIN a mano sin una
  librería de criptografía real habría sido peor que usar una ya revisada, aun para un
  PIN corto.
- **El desbloqueo del PIN es un `Arc<AtomicBool>` en memoria, no en SQLite ni en el
  keychain**: es exactamente lo que pide el control parental (se vuelve a pedir el PIN
  al reiniciar la app), y evita el riesgo de guardar "desbloqueado" por error donde no
  tocaba.
- **Cambiar o quitar un PIN exige haberlo desbloqueado antes** (si ya existía uno): así
  no basta con tener la app abierta para quitárselo a otra persona sin saberlo.
- **`GET /api/parental/unlock` nunca distingue "no hay PIN" de "PIN equivocado"** en la
  respuesta: los dos casos dan el mismo 400, para no revelar por accidente si hay un PIN
  configurado a quien solo está probando códigos.

**Verificado contra los datos reales del usuario** (sin tocar ninguna credencial): tras
el arreglo de directo-excluido-de-continuar-viendo, `GET /api/home` devuelve su
película real en curso ("El día de la revelación", 2376/8834 s) con título correcto, y
ya no arrastra sus canales de zapping de pruebas anteriores.

**Deuda técnica pendiente.**
- Toda la de fases anteriores.
- Sin UI para marcar/corregir a mano qué categoría es adulta (ver decisión arriba).
- Favoritos sin conmutador en las filas de canal de `LiveScreen` (sí en tarjetas de
  película/serie y en sus fichas, que es donde `docs/PROMPTS.md` lo pide literalmente:
  "tarjeta y ficha" — un canal no tiene "ficha" en esta app).
- Dos tests salieron intermitentes una vez cada uno al correr la batería completa
  (pasan siempre en solitario): `tier_two_remuxes_a_live_ts_channel_to_fragmented_mp4`
  (F6, contención de puertos/procesos ffmpeg) y `seeded_db()` en `server::api`
  (contención de ficheros SQLite temporales). Ninguno es una regresión real, los dos
  apuntan a lo mismo: mucho test en paralelo compitiendo por puertos/ficheros
  temporales. Pendiente de mirar si conviene bajar el paralelismo de `cargo test` en
  este proyecto o dar más unicidad a los recursos temporales.

**Segunda vuelta tras probarlo el usuario: tres fallos reales.**
1. **La búsqueda de series enseñaba un resultado por episodio, no uno por serie**
   ("La Casa de Papel S4 E5", "S1 E3"... como entradas de serie sueltas). Causa real:
   el patrón de episodio `S\d+E\d+` de `catalog::normalize`/`sources::m3u` exigía los
   dígitos pegados a la "E" (`S04E05`); este proveedor real usa espacio (`S4 E5`), así
   que el título nunca se limpiaba, cada episodio sacaba un `match_key` distinto, y cada
   uno acababa siendo una fila de `series` por sí solo. Arreglado añadiendo `\s*` entre
   número y "E" en las dos copias del patrón (con test que confirma que "S4 E5" y
   "S4 E7" normalizan al mismo título). El catálogo ya sincronizado no se corrige solo:
   hace falta resincronizar para que `delete_orphaned_movies_and_series` limpie las
   filas de serie huérfanas que deja el `match_key` nuevo — mismo mecanismo que ya
   documentó F3 para el mismo tipo de problema.
2. **No había forma de ver la lista de favoritos en ningún sitio.** El conmutador
   cambiaba, pero `docs/PROMPTS.md` nunca decía explícitamente "y una pantalla para
   verlos" — un descuido real, no una decisión consciente. `GET /api/favorites` ahora
   devuelve título y portada (antes solo `item_kind`/`item_id`), y hay una pantalla
   nueva **Favoritos** en el menú, agrupada por película/serie/canal.
3. **"Siguiente episodio" a veces reanudaba a mitad en vez de empezar desde el
   principio.** `list_next_episodes` calculaba bien cuál era el episodio siguiente, pero
   no comprobaba si ese episodio ya tenía su propio progreso guardado (tocado fuera de
   orden en algún momento, por ejemplo probando F6). Al abrirlo, el reproductor hacía
   exactamente lo que tiene que hacer con progreso guardado: reanudar donde se dejó — el
   fallo no estaba ahí, sino en sugerir como "para empezar de cero" algo que no lo
   estaba. Arreglado excluyendo de la fila cualquier candidato con progreso propio ya
   guardado.

---

## 2026-08-31 — F9 mpv, catch-up y pulido

**Construida de manera autónoma.** Nivel 3 de la cascada (mpv), catch-up/timeshift,
mensajes de error en español y un registro de diagnóstico exportable. **Dos piezas no
se han podido verificar contra software real** (ver deuda técnica): no hay mpv
instalado en la máquina donde se ha escrito esto, y ningún canal del proveedor real del
usuario tiene `has_catchup=1` en los datos vistos hasta ahora.

**Qué se ha construido.**
- **Nivel 3 — mpv en ventana propia**: `media::mpv::spawn_mpv` lanza mpv con
  `--input-ipc-server`, `--user-agent` y `--force-window=yes`, con la URL *real* del
  proveedor, no la del proxy de nivel 2 — mpv existe precisamente para lo que el
  `-c:v copy` de ffmpeg no puede remuxear sin recodificar. Nuevo endpoint
  `POST /stream/mpv/:kind/:id`: responde `204` en cuanto mpv arranca (no cuando
  termina) y registra `play_tier=3`. Como aquí no hay cuerpo de respuesta HTTP al que
  atar la vida de la sesión (a diferencia de nivel 1/2), `StreamGuard` gana
  `run_until_exit_or_replaced`, que mantiene `is_busy()` a `true` sondeando el proceso
  cada 500 ms hasta que mpv cierra su ventana o llega una sesión más nueva. El
  frontend ofrece un botón **"Abrir con mpv"** solo cuando el backend contesta 501
  (códec no compatible con la ventana de la app). No se ha implementado la escalada
  automática "dos fallos de nivel 2 → mpv" que menciona `PROMPTS.md`: sin mpv real que
  probar, automatizar esa transición a ciegas parecía peor que dejarla manual.
- **Catch-up y timeshift**: `Source` gana `catchup_url`/`catchup_url_fallback` (por
  defecto "no soportado"; M3U no los sobreescribe, no hay forma estándar de pedirlo).
  `XtreamSource` construye las dos formas que documenta `docs/XTREAM-API.md` — la de
  segmentos de ruta (`/timeshift/user/pass/{min}/{fecha}/{id}.ts`) y la de query string
  (`/streaming/timeshift.php?...`) —, reutilizando `util::civil_time` (antes solo del
  EPG) para formatear la fecha `YYYY-MM-DD:HH-MM` en UTC. Nuevo
  `GET /stream/catchup/:channel_id?start=&duration_min=`: comprueba
  `channels.has_catchup` y que `start` caiga dentro de `channels.catchup_days` *antes*
  de construir ninguna URL o abrir ninguna conexión (regla R1); si está soportado,
  reutiliza la misma sonda/caché/cascada de nivel 1-2 que el directo. La Guía abre este
  endpoint en vez del directo al pulsar un programa ya emitido en un canal con
  catch-up; el resto de celdas (en curso, futuro, o canal sin catch-up) siguen
  llevando al directo tal cual, y es el backend quien rechaza con un mensaje claro si
  de verdad no se puede.
- **Mensajes de error en español**: `StreamError` reescrito con texto pensado para
  enseñarse tal cual al usuario ("la cuenta caducó, revisa tu suscripción", "puede que
  hayas alcanzado el límite de conexiones simultáneas..."), incluyendo una
  comprobación explícita de `expires_at` antes de intentar conectar. El frontend
  (`PlayerScreen`) ya no se conforma con el `onError` mudo de `<video>`: hace un
  `fetch` puntual de diagnóstico a la misma URL de stream (la sesión ya falló, así que
  esto no compite con nada) y enseña el texto real que dio el backend.
- **Registro de diagnóstico exportable**: `util::diag_log::DiagnosticsLog` es un
  segundo `tracing_subscriber::fmt::layer()` que copia en un anillo de memoria (2000
  líneas como máximo, regla R4) el mismo texto ya formateado que ve la consola — no
  vuelve a redactar nada porque no hace falta: cada sitio que loguea una URL de
  proveedor ya pasa por `redact()` antes de que la línea exista (invariante de R5 que
  ya cumplía el código desde F1). Nuevo comando de Tauri `export_diagnostics_log`
  vuelca el contenido a `<datos de la app>/diagnostico.log` y un botón en Ajustes
  enseña la ruta resultante.
- **Repaso de rendimiento**: revisado sin encontrar nada que corregir. Las listas de
  catálogo (películas, series, canales) ya paginan con `useInfiniteQuery` o
  virtualizan (`VirtualPosterGrid`/Directo) desde F4/F5; favoritos, continuar viendo y
  siguiente episodio son listas acotadas por diseño (unas pocas decenas de filas como
  mucho), no el catálogo completo, así que una consulta por ítem para enriquecerlas
  (ver F8) no es el problema de rendimiento que sería sobre 20-100k filas. Los índices
  que usan las consultas nuevas de F7-F9 (`idx_epg_lookup`, `idx_playback_recent`, PK
  de `channels`) ya existían desde `001_schema_inicial.sql`.

**Actualización 2026-08-31: mpv verificado contra la app real.** El usuario instaló
mpv (build de `shinchiro/mpv-winbuild-cmake`) y lo añadió al PATH de usuario. Con la
app corriendo de verdad (`npm run tauri dev`) se probó `POST /stream/mpv/channel/:id`
contra dos canales reales del catálogo: la ventana de mpv se abre con el título
correcto, cambiar de canal cierra sola la ventana anterior sin dejar duplicados, y
cerrar mpv a mano (simulando al usuario) no deja la app "pensando que sigue ocupada" —
un stream normal pedido justo después responde al instante. Sin ffmpeg ni mpv
huérfanos al terminar. Sigue sin probarse against un canal que *de verdad* necesite
nivel 3 por códec (HEVC, etc.): la prueba forzó el endpoint directamente, que no mira
el códec real del canal — pero el ciclo de vida completo (lanzar, sustituir, cerrar)
ya está confirmado, que era la parte que dependía de este proyecto y no del proveedor.

**Deuda técnica que deja esta fase.**
- **Catch-up sin verificar contra un proveedor que lo ofrezca de verdad.** La
  construcción de las dos URLs sigue `docs/XTREAM-API.md` al pie de la letra y tiene
  tests unitarios, pero ningún canal visto hasta ahora en los datos reales del usuario
  tiene `has_catchup=1`, así que el camino feliz completo (probar de verdad qué forma
  de URL acepta el panel) sigue sin confirmar.
- **mpv no sabe pedir catch-up.** `POST /stream/mpv/:kind/:id` solo lanza el directo;
  si un canal con catch-up necesitara nivel 3, el botón "Abrir con mpv" no aparece en
  ese modo (se oculta a propósito) en vez de abrir el momento equivocado.

---

## 2026-08-31 — F10 Empaquetado y apertura a la LAN

**Construida de manera autónoma**, con dos decisiones consultadas primero (versión
manual en vez de actualizaciones automáticas; construir el modo LAN entero ahora en vez
de esperar a que exista un cliente real) y una tercera resuelta a medio camino (la
config de `externalBin` rompía `cargo check` — ver más abajo).

**Qué se ha construido.**

- **Sidecars (ffmpeg, ffprobe, mpv)**: `media::resolve_binary` ahora mira primero junto
  al propio ejecutable (`std::env::current_exe()`, con el ajuste de `deps/` bajo `cargo
  test`) antes de caer al `PATH`. Es la misma convención que usa `tauri-plugin-shell`
  internamente para sus sidecars (confirmado leyendo su código fuente, no de memoria:
  `Command::new_sidecar` hace justo esto) — deliberadamente **sin** añadir esa
  dependencia, que habría obligado a rehacer `StreamGuard`/`media::ffmpeg`/`media::mpv`
  sobre su modelo de proceso (`CommandChild` no tiene `try_wait`, solo `kill(self)` y
  eventos de `Stdout`/`Terminated` por canal) para ganar nada que no se pudiera resolver
  con una función de 20 líneas.
- **Los binarios no van en el repositorio** (~550 MB): `apps/desktop/src-tauri/binaries/`
  está en `.gitignore` salvo un `LEEME.md` con de dónde descargarlos, qué nombre exacto
  darles (`<nombre>-x86_64-pc-windows-msvc.exe`, el sufijo de triple que Tauri retira al
  empaquetar) y sus hashes SHA-256.
- **`bundle.active` rompía la compilación normal, no solo el empaquetado.** Primer
  intento: `tauri.conf.json` con `bundle.active: true` + `externalBin` apuntando a
  `binaries/`. Resultado real: `cargo check` (sin ninguna intención de generar un
  instalador) empezó a fallar en seco con "resource path ... doesn't exist" en cuanto
  faltaba uno de los tres binarios — el build script de `tauri-build` valida
  `externalBin` en **cualquier** compilación, no solo al empaquetar. Arreglado
  devolviendo `tauri.conf.json` a `bundle.active: false` (como estaba) y moviendo todo
  lo de empaquetado a `tauri.bundle.conf.json`, un fichero de config aparte que Tauri
  fusiona por encima del principal solo cuando se le pide explícitamente
  (`tauri build --config src-tauri/tauri.bundle.conf.json`, script
  `npm run build:installer`). `cargo check`/`test`/`tauri dev` vuelven a funcionar sin
  que los binarios existan siquiera.
- **Comprobación de que existen al arrancar**: nuevo `GET /api/playback/binaries` (lee
  el mismo `Option<PathBuf>` que ya se resolvía en F9) y un aviso en Ajustes si falta
  alguno — antes solo quedaba constancia en el log (`tracing::warn!`), que nadie mira en
  el día a día.
- **Migraciones aplicadas antes de servir, no solo al invocar un comando Tauri.**
  Encontrado revisando `server::serve` para decidir dónde leer el ajuste de LAN: no
  había ningún `open_connection_at` (el que sí aplica migraciones) en el arranque del
  servidor — dependía de que el frontend invocara `list_providers` u otro comando Tauri
  antes de que llegara la primera petición HTTP real, una carrera de verdad en una
  instalación nueva sin base de datos todavía, no solo teórica. Arreglado abriendo la
  conexión (con migraciones) una vez al principio de `serve()`, antes de construir
  ningún estado.
- **Modo LAN**: `app_settings` (clave/valor, un ajuste nuevo no necesita su propia
  migración) y `paired_devices` (`migrations/003_lan_pairing.sql`). El servidor sigue
  atado a un único socket (regla R2) — no hay un segundo `TcpListener` en `0.0.0.0` ni
  un reinicio al activar el modo LAN: el socket ya escucha en todas las interfaces
  siempre (`0.0.0.0` cubre loopback también), y lo que de verdad decide si una petición
  LAN se sirve o se rechaza es `server::lan::require_local_or_paired`, un middleware que
  se aplica sobre el router ya fusionado (dentro de la capa de CORS, para que un
  `OPTIONS` de preflight no tenga que pasar por la comprobación de token). Toda petición
  de loopback pasa igual que siempre, sin tocar nada de esto.
  - **Emparejamiento**: código de 6 dígitos en memoria (`util::pairing::PairingState`,
    igual de efímero que `ParentalGuard`), de un solo uso pero **una tecleada
    equivocada no lo quema** — se descubrió escribiendo el primer test que sí lo hacía
    fallar mal: quemar el código a la primera hace que emparejar un mando a distancia
    sea frustrante si te equivocas una vez, así que solo se consume al acertar o al
    caducar (5 minutos).
  - **Token por dispositivo**: 32 bytes de `rand` (nueva dependencia directa — ya vivía
    en el árbol de forma transitoria, esto solo la sube a explícita), solo se guarda su
    hash SHA-256 (regla R5, mismo criterio que `profiles.pin_hash`). Se entrega una
    única vez al canjear el código; revocar es `revoked_at = now()`, nunca se borra la
    fila (conserva el historial de quién tuvo acceso alguna vez).
  - **"StreamGuard distribuido"** no necesitó código nuevo: todo dispositivo, local o
    LAN, pasa por el mismo proceso y el mismo `StreamGuard` — es "distribuido" en el
    sentido de que el PC decide por todos, no en el de coordinar varios procesos (no
    hay varios procesos que coordinar en esta arquitectura, ver `docs/ARCHITECTURE.md`).
  - **IP local para enseñar junto al código**: truco de `UdpSocket` "conectado" a una IP
    pública (no manda ningún paquete de verdad, UDP no tiene apretón de manos; solo hace
    que el sistema operativo resuelva la interfaz de salida) — sin librería nueva, sin
    descubrimiento automático (mDNS, etc., que no está construido: el otro dispositivo
    necesita que se le diga la IP a mano).
- **`docs/API.md`**: primera versión completa de la API HTTP, pensada para el cliente de
  Fire TV que llega en una fase posterior.

**Deuda técnica que deja esta fase.**
- **Nada de esto se ha probado contra un segundo dispositivo real.** El cliente de Fire
  TV no existe todavía; todos los tests de `server::lan` prueban el middleware
  inyectando un `ConnectInfo` sintético (no loopback) contra el propio proceso — no
  contra tráfico de red de verdad llegando por LAN. Antes de confiar en esto con un
  dispositivo real, probar al menos: que la IP detectada es la correcta en una red con
  más de una interfaz (VPN, adaptador virtual de Docker/WSL, etc. — el truco del
  `UdpSocket` puede dar una interfaz "rara" en esos casos), y que el firewall de
  Windows no bloquea la conexión entrante por defecto (Windows suele preguntar la
  primera vez que un proceso escucha en una interfaz no loopback; no se ha podido
  probar ese diálogo).
- **Sin descubrimiento automático del dispositivo.** El futuro cliente de Fire TV
  necesita que se le teclee la IP:puerto a mano (o algún QR/mDNS futuro, no construido).
- **Control parental no es por dispositivo.** El desbloqueo (`ParentalGuard`) es un
  único booleano por proceso — un dispositivo LAN que desbloquee lo desbloquea para
  todos los que compartan esta app en ese momento, incluida la propia ventana de
  escritorio. Aceptable mientras el uso real siga siendo "una familia, un PC".
- **El instalador no está firmado.** Windows SmartScreen avisará de "editor
  desconocido" hasta que se firme (fuera de alcance sin un certificado de firma de
  código, que cuesta dinero y no tenía sentido comprar solo para probarlo en esta
  máquina).
- **Licencias de los sidecars.** Ver `apps/desktop/src-tauri/binaries/LEEME.md`: ffmpeg
  ("full build") y mpv son GPL. Empaquetarlos como sidecars (procesos aparte, nunca
  enlazados) es agregación, no obliga a que el resto de la app sea GPL — pero si el
  instalador se comparte con alguien más, hay que ofrecer/enlazar el código fuente
  exacto de esos binarios, y valdría la pena cambiar a un build **LGPL** de ffmpeg (esta
  app nunca necesita sus codificadores GPL, solo copia/remuxea).

---

## 2026-09-17 — Ronda de I+D post-F10: funciones "premium"

Con las diez fases del plan original cerradas, se pidió pensar como plataforma de
streaming real y añadir lo que la hiciera sentir menos como un cliente IPTV genérico y
más como Netflix/Plex — sin una fase de `PROMPTS.md` que lo enmarque, así que queda
documentado aquí de una vez en lugar de trocearlo. Todo lo de esta sección se construyó
y verificó de forma autónoma, incluyendo verificación en vivo contra la app real (cuenta
de prueba `claude-qa-test@vibeiptv.test`, perfil "QA Claude" — ver más abajo) una vez
resuelto un corte temporal del proyecto de Supabase (pausado por inactividad del plan
gratuito; el usuario lo reactivó desde su panel).

**Cuenta y perfiles.**
- Corregida una condición de carrera en `sync::session::get_valid()`: dos llamadas
  concurrentes a refrescar el token (Supabase rota el refresh token en cada uso) podían
  hacer que la perdedora borrase una sesión que la ganadora acababa de renovar
  legítimamente. Serializado con un `Mutex` y doble comprobación tras adquirirlo.
- `AccountGate` deja de tener un botón "Continuar sin cuenta": la única forma de
  saltárselo es un error de red genuino. Cerrar sesión ahora limpia también el perfil
  activo e invalida la caché de perfiles, así que un login nuevo siempre vuelve a
  preguntar "¿quién eres?" en vez de reutilizar el último perfil elegido.
- Los perfiles viven dentro de la cuenta conceptualmente (para eso es la sincronización),
  pero la tabla local `profiles` sigue siendo del PC, no de la cuenta activa — iniciar
  sesión con una cuenta de Supabase distinta en la misma máquina ve los mismos perfiles
  locales. Documentado como comportamiento esperado (ver "Deuda técnica" más abajo, no es
  un descuido).
- `ProfilesScreen` distingue "cuenta recién creada" (invita a crear el primer perfil
  directamente) de la rejilla habitual con "Administrar perfiles".

**Catálogo enriquecido con TMDB (bajo demanda, nunca en la sincronización masiva).**
- Tráiler de YouTube y clasificación por edad (`youtube_trailer_key`, `age_rating`,
  migración 006/007): se comprueban una sola vez por título, la primera vez que se abre
  su ficha, en una única llamada combinada a TMDB (`append_to_response`).
- Nombres de género reales (`genre_names`, poblada durante el emparejamiento TMDB) en vez
  de enseñar ids numéricos sueltos.
- Filas por género en Inicio, con los géneros que de verdad tienen más películas en el
  catálogo del usuario, no una lista fija.
- "Recomendado para ti" en Inicio: semillas = favoritos y películas vistas más del 70%,
  se le pregunta a TMDB por recomendaciones de cada semilla y el resultado se cruza
  contra el catálogo local (regla de oro: nunca sugerir algo que el proveedor no tiene).
  Caché en memoria de una hora con refresco en segundo plano (`catalog::recommendations`,
  sin tabla en SQLite a propósito: es barato de recalcular).
- "Más como esto" en la ficha de película y serie: mismo mecanismo pero a partir de un
  único título — se aprovechó el mismo `append_to_response` que ya pedía tráiler y
  clasificación (`recommendations` añadido a la misma llamada, cero peticiones nuevas a
  TMDB). Los ids recomendados se guardan una vez (migración 009,
  `similar_tmdb_ids`, texto separado por comas) pero el cruce contra el catálogo local se
  recalcula en cada visita, así un título añadido después también puede aparecer.
- `get_vod_info` de Xtream (duración y códecs reales del fichero, sin abrir el stream):
  parseo deliberadamente tolerante (`serde_json::Value`) porque no existe una muestra
  fiable de este endpoint en paneles reales — ver "Deuda técnica".
- Insignia "Nuevo" en cualquier tarjeta cuyo `added_at` sea de los últimos 7 días.

**Paginación, filtros y orden reales en Películas/Series.**
- Cursor compuesto `(columna_de_orden, id)` en vez de solo `id` para poder ordenar por
  título, año o nota y seguir paginando de forma exacta — antes solo funcionaba bien con
  el orden por defecto. Se encontró y arregló un bug real donde el cursor de continuación
  no se rellenaba para los órdenes por año/nota (probado con un test HTTP de extremo a
  extremo, no solo unitario).
- `CatalogFilterBar`: género, año, calidad y orden, con selects nativos y botón "Quitar
  filtros". El backend ya aceptaba estos parámetros desde F4 pero no había interfaz.
- Selector de fuente/calidad en la ficha de película cuando hay más de una disponible.

**Interacción con lo ya visto.**
- Quitar un título de "Continuar viendo" a mano (borra el progreso entero, no lo oculta).
- Checkmark de episodio visto en la ficha de serie, ahora también clicable para marcar o
  desmarcar sin reproducir (como Plex/Emby) — antes solo reflejaba lo ya visto de
  verdad. "Marcar temporada como (no) vista" junto a las pestañas de temporada. Ambos
  reutilizan `playback_state`/`upsert_playback_progress`: marcar a mano no es un mecanismo
  aparte, solo posición = duración.

**Descubrimiento.**
- Botón "Sorpréndeme" en la barra lateral: título al azar de todo el catálogo (`LIMIT 1
  OFFSET` a un desplazamiento al azar, nunca `ORDER BY RANDOM()` sobre un catálogo de
  hasta 100.000 filas), con preferencia por títulos ya emparejados con TMDB.
- Búsqueda con portadas reales en vez de una lista de texto, con navegación por flechas
  desde el campo de texto al primer resultado.

**Navegación por teclado pensando en mando a distancia.**
- Cada `Row` ya sabía moverse en horizontal; ahora arriba/abajo salta a la fila anterior/
  siguiente de toda la pantalla (marcando cada pista con `data-row-track`), manteniendo
  la misma columna cuando puede. La barra lateral gana su propia navegación vertical y
  una flecha derecha que entra al primer elemento enfocable del contenido. Directo y
  Guía ya tenían navegación completa por flechas desde F7, no hizo falta tocarlos.
  Alcance deliberado: no se intentó un sistema de navegación espacial 2D genérico entre
  regiones arbitrarias de cualquier pantalla — más grande y arriesgado de acertar sin
  hardware de mando real para probarlo.
- Recordar el scroll y los filtros de Películas/Series al volver de una ficha (se
  desmontan al navegar fuera): `lib/screenMemory.ts`, un `Map` en memoria de sesión, no
  `localStorage` — es estado de UI efímero, no algo que deba sobrevivir a un reinicio.

**Coherencia visual.** Repaso acotado a propósito: los botones "Reproducir"/"Más
información" del banner de Inicio usaban un radio de esquina distinto al mismo par de
botones en las fichas — corregido. Los formularios (cuenta, ajustes, perfiles) y los
controles compactos de filtro/guía tienen su propio registro visual consistente puertas
adentro y se dejaron tal cual: normalizarlos habría sido mucho más arriesgado para un
beneficio dudoso.

**Verificación en vivo (no solo tests).** Se creó una cuenta de Supabase de prueba
(`claude-qa-test@vibeiptv.test`) y un perfil "QA Claude" para poder entrar de verdad a la
aplicación (antes de esto, `AppLayout` vive detrás de `AccountGate`/`ActiveProfileGate` y
no había credenciales disponibles). Con acceso real se confirmó en pantalla: navegación
por flechas entre filas, "Sorpréndeme", "Más como esto", marcar episodio/temporada como
visto, favoritos (añadir y quitar), búsqueda, Ajustes, Directo y Guía — todo funcionando
sin fallos. Se detectaron canales y alguna película duplicados en el catálogo del
proveedor de pruebas (mismo nombre, `remote_id` distinto); comprobado contra la base de
datos que son feeds de respaldo reales del propio proveedor, no un fallo de
sincronización — no se tocó, deduplicar por nombre escondería alternativas legítimas.

**Deuda técnica que deja esta ronda.**
- **`get_vod_info` sin fixture de un panel real.** El parseo tolerante nunca se ha
  probado contra la respuesta real de un proveedor de producción, solo contra una
  muestra sintética anonimizada. Si un panel real da una forma distinta, fallará en
  silencio (vuelve a `VodInfo::default()`), no con un error visible.
- **Perfiles locales no aislados por cuenta remota.** Ver más arriba — iniciar sesión con
  una cuenta de Supabase distinta en el mismo PC ve los mismos perfiles locales. Correcto
  para "una familia, un PC", pero si algún día se comparte un PC entre cuentas sin
  relación, haría falta reconsiderarlo.
- ~~**Catálogo del proveedor con duplicados reales.**~~ Resuelto después de cerrar esta
  ronda: `LiveScreen` agrupa los canales por nombre exacto sobre todo lo ya cargado
  (`groupChannelsByName`, sin tocar la paginación del backend — agrupar en SQL entre
  páginas era mucho más frágil) y muestra un selector "Fuente N" solo cuando el grupo
  tiene más de un `remote_id`. Verificado en vivo: "24 Horas"/"La 1"/"La 2" pasan a
  aparecer una sola vez, con el selector; "Al Jazeera" (sin duplicados) no lo enseña.
- **Navegación espacial por teclado incompleta a propósito.** Cubre Inicio y las filas de
  ficha; no cubre transiciones entre el borde de una rejilla virtualizada (Películas/
  Series) y la barra de filtros o la barra lateral.

**Añadido después de cerrar esta ronda: selector de pista de audio.** El plan original de
F6 pedía "pistas de audio, subtítulos" en el reproductor y nunca se llegó a construir —
hueco real del plan, no una función "premium" inventada. Recomendaciones de series y
tendencias (más arriba) sí quedaron probadas en vivo contra la app real; esto no ha
podido probarse igual, y por qué queda documentado aquí en vez de asumirlo verificado.

- `media::probe::AudioTrack` (índice relativo entre pistas de audio, idioma si el
  proveedor lo etiqueta, códec, canales) y su lista completa en `ProbeResult` — antes
  `parse_ffprobe_json` solo se quedaba con la primera pista de audio, para
  `decide_tier`/`audio_needs_transcode`, que lo siguen usando igual.
- Persistido en `probe_cache.audio_tracks_json` (migración 010) como texto, mismo
  criterio que `similar_tmdb_ids` (migración 009): es una lista pequeña y de solo
  lectura, no hace falta ni una tabla propia ni una sonda aparte — se calcula la única
  vez que ya se inspecciona el stream (regla R1).
- `GET /stream/:kind/:id` acepta `?audio_track=N`; en nivel 2 (remux) se traduce a
  `-map 0:v:0 -map 0:a:N` en `media::ffmpeg::build_remux_command`. Sin el parámetro,
  ffmpeg sigue decidiendo solo qué pistas incluir, exactamente como antes de que
  existiera el selector — no toca el caso común.
- **Deliberadamente fuera de alcance: nivel 1 y subtítulos.** En nivel 1 (proxy directo)
  esta app no decide qué pistas lleva el fichero — sería el propio `<video>` quien tendría
  que exponer `audioTracks` de forma nativa, sin verificar si el WebView lo soporta bien
  de verdad. Los subtítulos incrustados (MKV) exigirían abrir una segunda conexión al
  proveedor para extraerlos con ffmpeg aparte del remux en curso — **eso violaría
  directamente la regla R1** (una sola conexión de stream activa), así que no se ha
  intentado ninguna forma de extraerlos.
- **Verificación real, con los límites que tiene.** Se añadió un test funcional
  (`selecting_an_audio_track_maps_that_exact_track_into_the_remux`,
  `server::stream::tests`) que genera un stream sintético con dos pistas de audio
  distinguibles (mono/estéreo) y confirma con un segundo `ffprobe` que el remux con
  `audio_track=1` lleva de verdad la estéreo y ninguna otra — no es una prueba contra un
  proveedor real, pero sí confirma el mecanismo de principio a fin con ffmpeg de verdad,
  no solo que los argumentos se construyen bien. Lo que **no** se ha podido comprobar: si
  el contenido real de la cuenta de prueba trae más de una pista de audio, y si el
  selector se comporta bien en un canal en directo cambiando de pista en caliente
  (`?audio_track=` fuerza un remux nuevo, mismo mecanismo que cambiar de fuente/calidad,
  pero sin un canal real de varios idiomas a mano para probarlo).

---

## 2026-10-15 — Nueva hoja de ruta (F11–F16)

Con F0–F10 y la ronda "premium" cerradas, se revisó todo el repositorio y se añadieron
seis fases a `PROMPTS.md`. Decidido con el usuario:

- **F11 primero, por urgencia.** El proyecto gratuito de Supabase se pausaba tras unos 7
  días sin uso. Peor que la pausa: con la cuenta obligatoria (`26bc5ab`), el refresco del
  token fallaba con un error HTTP (no de red), `sync::session` borraba la sesión y
  `AccountGate` pedía un login que también fallaba. **La app quedaba inutilizable aunque
  catálogo, base de datos y streaming estuvieran en el PC.** Además, cada cambio se
  enviaba una sola vez, sin reintento: lo hecho durante un corte nunca llegaba a la nube.
- **Firebase en lugar de Supabase**, frente a Supabase Pro (25 $/mes), Neon (Auth en beta,
  sin cambio de contraseña gestionado, SMTP propio para recuperar) y Cloudflare Workers +
  D1 (autenticación propia). Comparativa completa en `docs/CLOUD-SYNC.md`.
- **iOS (F15) antes que Fire TV (F16)**: el usuario no tiene un Fire TV a mano.
- **Repositorio privado en GitHub** como copia de seguridad: hasta ahora el código solo
  existía en este PC (sin remoto).

**Entorno.** La máquina ya no tenía el toolchain de Rust ni las Build Tools de Visual
Studio (los dos estaban en F0). Reinstalados con autorización del usuario: `rustup`
(estable, `x86_64-pc-windows-msvc`) y VS 2022 Build Tools con la carga de trabajo C++.

**Línea base antes de tocar nada:** 406 tests pasaban y 2 fallaban de forma
intermitente (`sync::session`), porque varios tests usaban a la vez la misma entrada real
del keychain. Al ejecutarlos, además, borraban la sesión real de la app de quien
estuviera desarrollando.

---

## 2026-10-15 — F11 Nube sin pausas y app local-first

**Qué se ha construido.**
- **Proyecto Firebase `vibe-iptv-sync`**, creado y configurado entero desde la CLI (ya
  tenía sesión del usuario): app web, Firestore en `europe-southwest1`, login por correo y
  reglas de seguridad. Todo versionado en `infra/firebase/` y desplegable con
  `firebase deploy --only firestore,auth`.
- **`sync::backend::CloudBackend`**, un trait como `sources::Source`: el motor no sabe
  nada de Firebase. `sync::firebase::FirebaseBackend` (Auth REST + Firestore REST, sin SDK)
  es la única implementación real; `sync::memory::MemoryBackend` (solo tests) simula
  cortes y cambios desde otro dispositivo.
- **Cola de salida persistente** (`sync_outbox`, migración 011, `db::cloud`,
  `sync::outbox`). Cada cambio se encola en la misma conexión que lo guarda, con
  sustitución por documento, revisión contra carreras, espera exponencial y progreso
  agrupado (como mucho una escritura por título y minuto, antes una cada 10 s).
- **Motor en segundo plano** (`sync::engine`): primera fusión por cuenta (sube también el
  progreso local, que con Supabase nunca se subía), envío de la cola y **bajada
  incremental cada 3 minutos** con borrados lógicos. Antes los cambios de otro dispositivo
  solo llegaban al volver a iniciar sesión.
- **Local-first de verdad.** La sesión solo se borra si Firebase rechaza el token de
  refresco. `GET /api/account/status` ya no toca la red. Un fallo pasajero al entrar
  responde 503 y `AccountGate` ofrece "Entrar sin conexión". Ajustes muestra el estado
  ("Todo sincronizado", "N cambios esperando", "Sincronización en pausa: ...").
- **Recuperar contraseña**: el correo de Firebase abre una página en el navegador para
  fijar la nueva; pegar el enlace en la app sigue funcionando (`oobCode`).
- `sync::supabase` eliminado. La sesión de Supabase que quedara en el keychain se borra
  al arrancar.

**Decisiones tomadas.**
- **Colecciones planas** (`users/{uid}/favorites`, no dentro de cada perfil): la bajada
  incremental es una consulta simple por colección con el índice automático de un campo,
  sin consultas de grupo de colecciones ni índices compuestos.
- **Marca de tiempo del servidor para el cursor y del cliente para los conflictos.** El
  cursor no puede depender de relojes de clientes distintos; quién gana un conflicto sí
  tiene que reflejar cuándo lo hizo el usuario.
- **Ids de perfil generados en el cliente** (20 alfanuméricos, como Firestore): permiten
  encolar sin conexión. Los UUID de Supabase que ya había se reutilizan tal cual, así
  que la migración no cambia ningún id local.
- **La fusión de lo bajado conserva fechas** (`put_playback_from_remote`,
  `upsert_profile_from_remote`) en vez de reutilizar las funciones de escritura de la
  API, que ponen la hora actual: si no, cada bajada parecería un cambio local nuevo y
  rebotaría a la nube.
- **`busy_timeout` de 5 s en todas las conexiones SQLite.** Con WAL hay un único escritor;
  sin espera, el segundo fallaba al instante con `SQLITE_BUSY`. Ahora el motor escribe en
  segundo plano mientras la API guarda progreso, y la colisión deja de ser rara.
- **Entrada de keychain propia para los tests** (`cloud-session-test`), para no cerrar
  la sesión real al ejecutar `cargo test`, y **un único cerrojo para todo test que toque
  el keychain real** (`util::keychain::TEST_LOCK`). Arregla los dos fallos intermitentes
  de la línea base y otro que apareció después: los propios tests de `util::keychain`
  fallaban 2 de cada 5 ejecuciones (`NoEntry` justo después de guardar, con entradas
  distintas). El Administrador de credenciales de Windows no tolera bien varias
  escrituras y lecturas simultáneas desde el mismo proceso.
- **Ninguna dependencia nueva.** `reqwest`, `sha2`, `rand`, `async-trait` y `serde_json`
  ya estaban.

**Bug real encontrado solo contra Firebase de verdad.** Con el backend falso todo pasaba,
pero la prueba de punta a punta contra el proyecto real no conseguía sesión tras el
login. Causa: el Administrador de credenciales de Windows admite 2560 bytes por secreto,
`keyring` los guarda en UTF-16, y una sesión de Firebase completa ocupa unos 2644 (el
token de acceso es un JWT de ~950 caracteres). `keyring` devolvía `TooLong`, se
registraba un aviso y la sesión nunca se guardaba. Con Supabase los tokens eran más
cortos y nunca pasó. Arreglado guardando en el keychain solo el token de refresco, el uid
y el correo (~600 bytes); el token de acceso, que dura una hora, vive en memoria y se
renueva al arrancar. Test de regresión con tamaños reales.

**Verificación real.**
- `cargo test`: 426 pasan, 0 fallan, 3 ignorados (los dos de abajo y el de zapping de
  F6). Cinco ejecuciones seguidas, sin intermitencias. `tsc --noEmit` y `vite build`
  limpios.
- Contra el proyecto real, con una cuenta de pruebas (`claude-qa-test@vibeiptv.test`, su
  contraseña no está en el repositorio):
  `live_roundtrip_against_the_real_project` (login, refresco, escritura, bajada
  incremental, borrado lógico, error de contraseña) y
  `live_end_to_end_sync_through_the_local_api` (API local → cola → motor → Firestore,
  sobre una base de datos temporal). Las dos pasan en unos 4 s.
- Reglas comprobadas a mano con `curl`: otro uid y una petición sin token reciben 403.

**Deuda técnica pendiente.**
- **La interfaz no se ha visto con datos reales.** Ni "Entrar sin conexión" ni la línea
  de estado de Ajustes se han probado en la ventana de la app, solo su lógica por la API.
- **Migración del usuario sin hacer.** Al abrir esta versión, la sesión de Supabase ya no
  vale: hay que crear la cuenta en Firebase desde la pantalla de cuenta, y la primera
  fusión sube todo lo que haya en el PC. **Pero en este PC no quedaba nada:** al arrancar
  la app real para comprobar F11, su base de datos se creó vacía (0 proveedores, 0
  perfiles), igual que faltaban Rust y Visual Studio. Lo que hubiera en Supabase
  (perfiles, favoritos y progreso; nunca el catálogo ni el proveedor) no se migra solo.
  Hace falta volver a dar de alta el proveedor, y decidir si merece la pena importar lo
  de Supabase.
- El proyecto de Supabase ya no lo usa nada. No borrarlo hasta decidir lo anterior.
- Ver "Límites conocidos" en `docs/CLOUD-SYNC.md` (lo que el catálogo local no tiene al
  bajar se pierde para este PC; conflictos por reloj del cliente).

---

## Deuda técnica conocida desde el diseño

- **mpv en ventana separada.** El nivel 3 de la cascada no permite superponer la
  interfaz sobre el vídeo. Empotrarlo requiere manejo de handles nativos de ventana y
  composición transparente. Aceptado para v1 porque afecta a una minoría de contenido.
- **Fusión de títulos homónimos.** Dos películas distintas del mismo año con el mismo
  título se agrupan bajo una ficha. Se corrige parcialmente con el emparejamiento TMDB.
- **Sin soporte oficial de macOS.** Windows primero, Linux si sale gratis.
