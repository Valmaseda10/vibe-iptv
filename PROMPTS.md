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
- [x] F11 Nube sin pausas y app local-first (Firebase en lugar de Supabase — verificada contra el proyecto real; la interfaz nueva sin ver en pantalla, ver DECISIONS.md)
- [x] F12 Cerrar la deuda de v1 (logos en disco, hls.js y LGPL quedan como pendientes documentados)
- [x] F13 Directo de nivel TiviMate
- [ ] F14 Detalles premium en VOD y perfiles
- [ ] F15 Cliente iOS
- [ ] F16 Cliente Fire TV (pospuesto: no hay dispositivo a mano para probarlo)

Las fases F11 en adelante se añadieron el 2026-10-15, con las diez primeras ya cerradas.
Ver la entrada de esa fecha en `docs/DECISIONS.md` para el porqué del orden.

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

## F11 — Nube sin pausas y app local-first

```
La nube (cuenta, perfiles, favoritos y progreso entre dispositivos) pasa de Supabase a
Firebase, y deja de poder bloquear la app. Lee docs/CLOUD-SYNC.md entero.

- La cuenta nunca bloquea el uso local: un servidor caído, pausado o sin red no borra
  la sesión ni deja al usuario fuera de su propio catálogo. Solo una sesión revocada de
  verdad vuelve a pedir login. Si el login falla por no poder contactar con la nube,
  se ofrece entrar sin conexión
- Cola de salida persistente (sync_outbox) con reintentos y espera exponencial: ningún
  cambio hecho sin conexión se pierde. El progreso se agrupa (como mucho una escritura
  por título y minuto) en vez de una petición cada 10 s
- Trait CloudBackend, con FirebaseBackend (Auth REST + Firestore REST) como única
  implementación real y un backend en memoria para tests
- Bajada incremental periódica por marca de tiempo del servidor, con borrados lógicos,
  para que los cambios de otro dispositivo lleguen sin cerrar sesión
- Fusión inicial al iniciar sesión que sube también el progreso local, no solo
  favoritos y perfiles
- Supabase se retira por completo
- Configuración de Firebase como código en infra/firebase/

Criterio de aceptación: con la red cortada la app abre y se usa con normalidad; al
volver la red, lo hecho sin conexión aparece en Firestore. Un cambio hecho desde otro
cliente aparece en el PC en menos de 5 minutos sin cerrar sesión.
```

---

## F12 — Cerrar la deuda de v1

```
Lo que docs/DECISIONS.md deja pendiente o sin verificar, priorizado por lo que más se
nota al usarla.

- Subtítulos incrustados de texto (SRT, ASS, mov_text) como segunda salida del MISMO
  proceso ffmpeg del remux, convertidos a WebVTT: una sola conexión al proveedor (R1)
- Escalada automática de nivel 2 a mpv tras dos fallos seguidos
- Refresco periódico de la EPG en segundo plano, respetando StreamGuard
- Caché en disco de logos de canal servida por /img
- Favorito de canal desde Directo; marcar y desmarcar categorías adultas a mano
- hls.js para proveedores que sirvan .m3u8, con reescritura del manifiesto en el proxy
- ffmpeg en build LGPL y comprobación de firma del instalador

Criterio de aceptación: una película MKV con subtítulos SRT incrustados los muestra y
permite elegirlos, sin abrir una segunda conexión al proveedor.
```

---

## F13 — Directo de nivel TiviMate

```
- Pausar y rebobinar el directo con un búfer en disco de la sesión ya abierta (nada de
  conexiones extra), con límite de tamaño y borrado al cambiar de canal
- Canales recientes y salto por número de canal
- Recordatorios de programa: aviso cuando empieza, con un clic para ir al canal
- Búsqueda dentro de la guía
- Guía virtualizada en los dos ejes, sin el límite de 60 canales

Criterio de aceptación: pausar un canal 10 minutos y reanudar donde se dejó, sin que el
proveedor vea más de una conexión.
```

---

## F14 — Detalles premium en VOD y perfiles

```
- Perfil infantil: solo contenido con clasificación por edad apta, sin directo adulto
- Control parental por perfil, no por proceso
- Velocidad de reproducción e imagen en imagen
- Saltar intro solo cuando el fichero traiga capítulos (ffprobe), nunca inventado

Criterio de aceptación: un perfil infantil no puede llegar a ningún título no apto ni
por búsqueda, ni por recomendaciones, ni por "Sorpréndeme".
```

---

## F15 — Cliente iOS

```
Primer cliente fuera del PC, en clients/ios. Lee docs/API.md y docs/CLOUD-SYNC.md.

- SwiftUI, reproducción con VLCKit (TS y MKV sin remux)
- Misma cuenta Firebase: perfiles, favoritos y progreso con el SDK oficial y caché offline
- Catálogo desde la API LAN del PC cuando está encendido, con emparejamiento por código
- Decidir en el plan de la fase si el iPhone habla con el proveedor directamente cuando
  el PC está apagado (implica credenciales del proveedor en el dispositivo: R5)

Criterio de aceptación: empiezo una película en el PC y la continúo en el iPhone desde
el mismo minuto.
```

---

## F16 — Cliente Fire TV

```
Pospuesto hasta tener un Fire TV para probar. Kotlin, Compose for TV y Media3
(ExoPlayer) contra la misma API LAN y la misma cuenta Firebase que el cliente iOS.
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
