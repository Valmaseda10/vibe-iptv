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

**Deuda técnica pendiente.**
- Toda la de fases anteriores.
- **Verificado por el usuario**: audio y vídeo van bien en directo y en película. La
  búsqueda en VOD remuxeado está corregida dos veces (ver arriba) pero **todavía sin una
  tercera confirmación del usuario** tras este último cambio.
- Series, zapping y reanudar no se han probado todavía con el proveedor real del
  usuario.
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

## Deuda técnica conocida desde el diseño

- **mpv en ventana separada.** El nivel 3 de la cascada no permite superponer la
  interfaz sobre el vídeo. Empotrarlo requiere manejo de handles nativos de ventana y
  composición transparente. Aceptado para v1 porque afecta a una minoría de contenido.
- **Fusión de títulos homónimos.** Dos películas distintas del mismo año con el mismo
  título se agrupan bajo una ficha. Se corrige parcialmente con el emparejamiento TMDB.
- **Sin soporte oficial de macOS.** Windows primero, Linux si sale gratis.
