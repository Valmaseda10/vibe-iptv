# Plan de fases y prompts para Claude Code

Una fase por sesión. Abre una sesión limpia en cada fase para no arrastrar contexto
inútil. Marca la casilla cuando la fase esté aceptada.

**Antes de empezar cada fase, pega esta línea:**

> Lee `CLAUDE.md`, `docs/ARCHITECTURE.md` y `docs/DATA-MODEL.md`. Después dime tu plan
> para la fase N antes de escribir código.

- [x] F0 Andamiaje
- [x] F1 Capa de fuentes
- [x] F2 Base de datos y sincronización
- [x] F3 Normalización y TMDB
- [x] F4 API local y tipos
- [x] F5 Cascarón de interfaz (primera versión, sin iterar contigo — ver DECISIONS.md)
- [x] F6 Reproductor (construido sin iterar contigo por partes como pide esta fase — ver DECISIONS.md)
- [x] F7 EPG y directo (construido de manera autónoma — ver DECISIONS.md)
- [x] F8 Búsqueda, favoritos, continuar viendo (construido de manera autónoma — ver DECISIONS.md)
- [x] F9 mpv, catch-up y pulido (construido de manera autónoma — mpv y catch-up sin verificar contra software/proveedor real, ver DECISIONS.md)
- [x] F10 Empaquetado y apertura a la LAN (construido de manera autónoma — modo LAN sin verificar contra un dispositivo real, instalador sin firmar, ver DECISIONS.md)

---

## F0 — Andamiaje

```
Vamos a arrancar el proyecto VIBE IPTV. Lee CLAUDE.md y docs/ARCHITECTURE.md antes de
tocar nada.

Crea el andamiaje de apps/desktop:
- Tauri 2 con React 19, TypeScript en modo strict y Vite
- Tailwind v4, TanStack Router y TanStack Query
- En Rust: tokio, axum, rusqlite con bundled y fts5, reqwest con rustls, serde,
  thiserror, tracing, keyring, ts-rs
- La estructura de carpetas exacta de la sección 4 de CLAUDE.md, con los módulos vacíos
  pero declarados
- Un servidor axum que arranca en 127.0.0.1 con puerto efímero, expone GET /api/health
  y comunica el puerto al frontend mediante un comando Tauri
- Un helper util::redact() que sustituye username, password y query strings en cualquier
  URL antes de registrarla, con tests
- Tema oscuro base y una pantalla de inicio que muestre el resultado de /api/health

Criterio de aceptación: `npm run tauri dev` abre una ventana que dice "núcleo ok" y el
puerto real. Nada de credenciales ni puertos fijos en el código.
```

---

## F1 — Capa de fuentes

```
Implementa el módulo sources siguiendo docs/XTREAM-API.md, que debes leer entero.

- El trait Source de docs/ARCHITECTURE.md
- XtreamSource: cuenta, categorías, canales, películas, series, detalle de serie,
  construcción de URLs de stream, User-Agent configurable, reintento sin el segmento
  /live/ si da 404
- M3uSource: parseo en streaming línea a línea, atributos de EXTINF, clasificación por
  ruta de URL con heurística de group-title como respaldo, lectura de x-tvg-url
- Detección de credenciales Xtream embebidas en una URL get.php
- Todos los campos numéricos tolerantes a llegar como cadena o ausentes

Tests con fixtures anonimizados en tests/fixtures/: una respuesta de cada endpoint
Xtream y una lista M3U de unas 50 entradas que cubra los casos raros. Inventa los
fixtures, no me pidas los míos.

Criterio de aceptación: los tests pasan y ningún log imprime credenciales.
```

---

## F2 — Base de datos y sincronización

```
Implementa la persistencia y el sincronizador.

- Migraciones SQL numeradas con el esquema exacto de docs/DATA-MODEL.md, aplicadas al
  arrancar. WAL y foreign_keys activados
- Módulo db con queries tipadas, sin SQL suelto por el resto del código
- StreamGuard según docs/ARCHITECTURE.md, con Drop que garantiza la liberación
- Sincronizador con los pasos 1 a 4 del documento: upsert por lotes de 500 dentro de
  transacción, cancelable, que comprueba StreamGuard::is_busy() antes de cada lote
- Progreso emitido al frontend por eventos de Tauri
- Contraseñas en el keychain vía keyring, nunca en SQLite
- Pantalla de ajustes con alta de proveedor, validación en vivo, y muestra de estado de
  cuenta, caducidad y conexiones máximas

Criterio de aceptación: añado mi proveedor real, sincroniza y puedo consultar por SQL el
recuento de canales, películas y series. 50.000 entradas en menos de 60 s.
```

---

## F3 — Normalización y TMDB

```
Implementa catalog/normalize.rs y catalog/tmdb.rs según la sección de normalización de
docs/DATA-MODEL.md.

- Las seis reglas en el orden indicado, con especial cuidado en la extracción de año:
  no debe destrozar títulos como "1917", "2012" o "Blade Runner 2049"
- Generación de match_key y agrupación de calidades bajo una ficha única
- Cliente TMDB: búsqueda con año, español con respaldo a inglés, umbral de similitud
  0,85, máximo 5 peticiones concurrentes, espera exponencial ante 429
- Caché persistente de aciertos Y de fallos en tmdb_cache
- Uso directo de tmdb_id si el proveedor lo proporciona
- Descarga de portadas y fondos a caché en disco, servidas por GET /img/*
- Todo esto en segundo plano, sin bloquear el catálogo

La clave de API de TMDB se pide en ajustes y va al keychain.

Criterio de aceptación: al menos el 80% de mis películas quedan con tmdb_state
'matched'. Escribe al menos 30 tests de normalización con títulos reales feos.
```

---

## F4 — API local y tipos

```
Expón el catálogo por HTTP en axum, todavía solo en 127.0.0.1.

GET /api/home                       filas: continuar viendo, recientes, por género
GET /api/live/categories
GET /api/live/channels              paginado, filtros de categoría y búsqueda
GET /api/movies                     paginado, filtros de género, año, calidad, orden
GET /api/movies/:id
GET /api/series                     paginado, mismos filtros
GET /api/series/:id                 dispara get_series_info bajo demanda si toca
GET /api/search?q=                  FTS5, agrupado por tipo
POST /api/playback/progress
GET/POST/DELETE /api/favorites

- Paginación por cursor, nunca offset con tablas grandes
- Todos los tipos de respuesta derivan ts-rs y se exportan a src/lib/api-types.ts
- Ninguna respuesta contiene credenciales ni URLs del proveedor: los streams se piden
  siempre por /stream/{kind}/{id}

Criterio de aceptación: curl a cada endpoint devuelve datos coherentes en menos de
100 ms con el catálogo lleno.
```

---

## F5 — Cascarón de interfaz

```
Construye la interfaz completa sin reproductor. Lee docs/PRD.md, requisito RF4.

Pantallas: Inicio, Directo, Películas, Series, Detalle de película, Detalle de serie,
Búsqueda, Ajustes.

- Navegación lateral que se colapsa, con iconos
- Filas horizontales desplazables con tarjetas de portada, carga diferida de imágenes y
  reserva de espacio para evitar saltos de layout
- Rejillas y listas virtualizadas con TanStack Virtual
- Detalle con fondo a sangre, degradado, sinopsis, metadatos y acción principal
- Detalle de serie con selector de temporada y lista de episodios
- Estados de carga con esqueletos, nunca con un spinner centrado
- Navegación completa por teclado desde ya: flechas para moverse entre tarjetas y filas,
  Enter para abrir, Escape para volver. Foco siempre visible

Diseño: tema oscuro, mucho contraste, tipografía grande, espaciado generoso. Que se vea
como una plataforma de streaming, no como un panel de administración. Enséñame primero
la pantalla de Inicio, la reviso, y luego seguimos con el resto.
```

---

## F6 — Reproductor

```
Implementa la cascada de reproducción de docs/ARCHITECTURE.md. Es la parte más delicada
del proyecto, así que hazla por partes y enséñame cada una funcionando.

Parte A: media/probe.rs con ffprobe como sidecar y caché en probe_cache. Decide el
nivel 1, 2 o 3 a partir de contenedor y códecs.

Parte B: GET /stream/:kind/:id en axum.
- Nivel 1: proxy con soporte de Range para el VOD, y hls.js en el cliente
- Nivel 2: ffmpeg remux a fMP4 con los flags exactos del documento, audio a AAC solo si
  es AC3, E-AC3 o DTS. Un proceso vivo por sesión, muerte confirmada del anterior antes
  de arrancar el siguiente
- Cierre limpio cuando el cliente corta la conexión

Parte C: interfaz del reproductor.
- Controles superpuestos que se ocultan tras 3 s de inactividad
- Barra de progreso con búsqueda, volumen, pistas de audio, subtítulos, pantalla completa
- Guardado de posición cada 10 s y al salir; terminado a partir del 90%
- Reanudar desde la posición guardada con opción de empezar de cero
- Siguiente episodio con cuenta atrás cancelable
- Zapping en directo con flechas arriba y abajo
- Panel de estadísticas oculto tras Ctrl+Shift+D: nivel usado, códecs, tasa de bits,
  buffer, tiempo hasta el primer fotograma

Criterio de aceptación: un canal de directo TS arranca en menos de 2 s, una película
MKV se reproduce con búsqueda funcional, y cambiar de canal veinte veces seguidas no
deja procesos ffmpeg huérfanos. Compruébalo.
```

---

## F7 — EPG y directo

```
Implementa la guía. Lee la sección de sincronización de docs/ARCHITECTURE.md.

- Descarga del XMLTV a fichero temporal, con soporte gzip
- Parseo en streaming con quick-xml, descartando lo anterior a ayer y lo posterior a
  siete días, inserción por lotes en transacción
- Emparejamiento entre channels.epg_channel_id y los ids del XMLTV, con informe de
  cuántos canales quedan sin EPG
- GET /api/epg/now para el programa actual y siguiente de una lista de canales
- GET /api/epg/grid con ventana temporal
- Lista de canales con programa actual y barra de progreso
- Vista de guía en rejilla, con desplazamiento temporal y navegación por teclado
- Superposición de "qué están dando" al cambiar de canal

Criterio de aceptación: un XMLTV de 30 MB se importa en menos de 30 s sin que la RAM
del proceso pase de 400 MB.
```

---

## F8 — Búsqueda, favoritos y continuar viendo

```
- Búsqueda instantánea con FTS5, sin diacríticos, con rebote de 150 ms, resultados
  agrupados por canales, películas y series
- Favoritos con conmutador en tarjeta y en ficha
- Fila de continuar viendo en Inicio, ordenada por reciente, que oculta lo terminado
- Fila de siguiente episodio pendiente por serie
- Perfil por defecto creado automáticamente
- PIN de control parental que oculta las categorías marcadas como adultas

Criterio de aceptación: escribir en el buscador da resultados en menos de 100 ms con el
catálogo completo cargado.
```

---

## F9 — mpv, catch-up y pulido

```
- Nivel 3 de la cascada: mpv como sidecar en ventana propia, controlado por IPC, usado
  cuando probe lo indica o cuando el nivel 2 falla dos veces. Registro de play_tier
- Catch-up y timeshift con las dos formas de URL de docs/XTREAM-API.md, ofrecido solo
  si tv_archive vale 1, limitado a tv_archive_duration días
- Repaso de rendimiento: revisa que no haya renderizados de más, listas sin virtualizar
  ni consultas sin índice
- Manejo de errores de cara al usuario: sin conexión, cuenta caducada, límite de
  conexiones alcanzado, stream caído. Mensajes en español que digan qué hacer
- Registro de diagnóstico exportable, pasado siempre por redact()
```

---

## F10 — Empaquetado y apertura a la LAN

```
- Instalador para Windows con los sidecars ffmpeg, ffprobe y mpv incluidos, y comprobación
  de que existen al arrancar
- Actualizaciones automáticas de Tauri, si me compensa mantener la infraestructura;
  si no, versión manual
- Modo LAN: el servidor axum pasa a escuchar en 0.0.0.0 solo si el usuario lo activa,
  con emparejamiento por código de seis dígitos y token por dispositivo, revocable
- El StreamGuard pasa a ser distribuido: el PC concede o deniega el permiso de stream
- Documenta la API en docs/API.md para el futuro cliente de Fire TV

Criterio de aceptación: instalo el .msi en un PC limpio sin nada preinstalado y funciona.
```

---

## Cómo repartir el trabajo

Fases 0 a 4 son núcleo: déjalas correr con poca supervisión, pero revisa los tests.
Fase 5 es de diseño: itera pantalla a pantalla, no aceptes la primera versión.
Fase 6 es donde se rompen las cosas: ve por partes y verifica cada una en tu proveedor
real antes de seguir.
Fases 7 a 9 son incrementales y admiten pausas largas entre ellas.

Si algo se atasca más de dos intentos, para, abre sesión nueva y da menos contexto pero
más preciso. Arrastrar una sesión larga y confundida es la forma más rápida de que
Claude Code empiece a inventar.
