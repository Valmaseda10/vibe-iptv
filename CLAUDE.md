# VIBE IPTV — Instrucciones permanentes para Claude Code

Este fichero se carga en cada sesión. Es la fuente de verdad sobre cómo trabajar en
este repositorio. Si algo aquí contradice una idea tuya, gana este fichero. Si crees
que este fichero está equivocado, dilo antes de escribir código, no después.

---

## 1. Qué estamos construyendo

Un reproductor IPTV de escritorio con experiencia de usuario equivalente a Netflix:
catálogo con portadas, filas por género, fichas de película y serie, continuar viendo,
buscador instantáneo, guía de programación y zapping rápido.

El contenido viene de proveedores IPTV del usuario (Xtream Codes y/o listas M3U).
La aplicación **no aloja ni distribuye contenido**: es un cliente, como VLC o Kodi.

**Objetivo v1: solo PC (Windows como plataforma principal, Linux si sale gratis).**
Fire TV, web e iOS vienen después y reutilizarán la API local que expone esta app.

---

## 2. Restricciones de diseño que no se negocian

Estas cinco reglas explican el 90% de las decisiones raras del proyecto. Respétalas.

### R1. Una sola conexión de stream activa
Las cuentas IPTV tienen un límite de conexiones simultáneas (típicamente 1 o 2). Si se
supera, el proveedor corta el stream o banea la cuenta temporalmente.

- Existe un `StreamGuard` global en Rust. Toda apertura de stream pasa por él.
- Antes de abrir un stream nuevo, el anterior se cierra y se espera confirmación.
- **Prohibido el prefetch de streams.** No se abre un stream para "sacar el thumbnail",
  ni para comprobar si funciona, ni para precargar el siguiente canal.
- La sincronización de catálogo y EPG se pausa mientras hay reproducción activa.

### R2. El usuario no tiene servidor
No hay VPS, ni NAS, ni Raspberry Pi. **La app de escritorio es el servidor.** Contiene
la base de datos, el sincronizador y un servidor HTTP local (axum). En v1 escucha solo
en `127.0.0.1`. En una fase posterior podrá escuchar en la LAN para que Fire TV se
conecte cuando el PC esté encendido. Diseña pensando en eso, pero no lo implementes aún.

**Única excepción: la nube de cuentas (Firebase, F11).** Perfiles, favoritos y progreso
viajan entre dispositivos por Firebase. Nunca el catálogo, ni credenciales o URLs del
proveedor, y la app tiene que seguir funcionando entera sin conexión con la nube: ningún
fallo de Firebase puede bloquear el uso local. Ver `docs/CLOUD-SYNC.md`.

### R3. Los códecs mandan
El directo IPTV llega en MPEG-TS crudo. El VOD en MKV o MP4. El WebView no reproduce
TS ni MKV de forma nativa, y no soporta audio AC3/E-AC3. Por eso existe la cascada de
reproducción de tres niveles descrita en `docs/ARCHITECTURE.md`. No intentes meter una
URL `.ts` directamente en un `<video>`: no va a funcionar.

### R4. El catálogo es enorme
Una lista típica tiene entre 20.000 y 100.000 entradas. Nada de cargar todo en memoria
ni de renderizar listas completas.

- Parseo en streaming, inserción en SQLite por lotes dentro de una transacción.
- Consultas siempre paginadas, con índices. Búsqueda con FTS5.
- Listas virtualizadas en la UI (TanStack Virtual).
- Objetivo: sincronización completa en menos de 60 s, apertura de la app en menos de 2 s.

### R5. Las credenciales no se registran nunca
Usuario, contraseña y URLs firmadas del proveedor no aparecen en logs, ni en mensajes
de error, ni en la telemetría de errores, ni en los nombres de fichero de caché.
Existe un helper `redact()` en Rust. Úsalo en todo log que toque una URL de proveedor.
Las credenciales se guardan en el keychain del sistema (`keyring` crate), nunca en
SQLite ni en un `.env` versionado.

---

## 3. Stack

| Capa | Tecnología | Por qué |
|---|---|---|
| Shell de escritorio | Tauri 2 | Binario de ~15 MB en vez de 200 MB de Electron |
| Núcleo | Rust (tokio, axum, rusqlite, reqwest) | Proceso de fondo, proxy de streams, parseo rápido |
| Base de datos | SQLite + FTS5 | Cero configuración, va en el appdata del usuario |
| UI | React 19 + TypeScript + Vite | Es el stack que el usuario ya domina |
| Estilos | Tailwind v4 | Iteración rápida de UI |
| Estado servidor | TanStack Query | Caché, revalidación, paginación infinita |
| Rutas | TanStack Router | Tipado, y encaja con el layout tipo TV |
| Reproducción | `<video>` + hls.js + remux ffmpeg + mpv como respaldo | Ver `docs/ARCHITECTURE.md` |
| Metadatos | TMDB API | Portadas, fondos, sinopsis, géneros |

Los binarios de `ffmpeg` y `mpv` se empaquetan como *sidecars* de Tauri en
`src-tauri/binaries/`. No asumas que están en el PATH del usuario.

---

## 4. Estructura del repositorio

```
vibe-iptv/
├─ CLAUDE.md                  ← este fichero
├─ PROMPTS.md                 ← plan de fases; consúltalo para saber qué toca ahora
├─ docs/
│  ├─ PRD.md                  ← alcance, qué está dentro y qué no
│  ├─ ARCHITECTURE.md         ← decisiones técnicas y flujo de datos
│  ├─ DATA-MODEL.md           ← esquema SQLite y reglas de normalización
│  ├─ XTREAM-API.md           ← referencia de los endpoints del proveedor
│  ├─ API.md                  ← API HTTP local, para los clientes de otras plataformas
│  ├─ CLOUD-SYNC.md           ← sincronización de cuentas con Firebase
│  └─ DECISIONS.md            ← registro de decisiones (ADR ligero)
├─ infra/firebase/            ← reglas y configuración de Firebase (firebase deploy)
└─ apps/desktop/
   ├─ src/                    ← React
   │  ├─ routes/              ← una carpeta por pantalla
   │  ├─ features/            ← home, live, movies, series, player, search, settings
   │  ├─ components/          ← componentes reutilizables tontos
   │  ├─ lib/                 ← cliente de API, hooks, formateadores
   │  └─ styles/
   └─ src-tauri/
      ├─ migrations/          ← NNN_descripcion.sql, aplicadas al arrancar (ver DATA-MODEL.md)
      ├─ src/
      │  ├─ main.rs
      │  ├─ server/           ← axum: rutas de API + proxy de stream
      │  ├─ sources/          ← xtream.rs, m3u.rs, trait Source
      │  ├─ catalog/          ← sync.rs, normalize.rs, tmdb.rs, epg.rs
      │  ├─ db/                 ← schema.rs, queries.rs, conexión (las migraciones viven en ../migrations/)
      │  ├─ media/            ← ffmpeg.rs, mpv.rs, probe.rs
      │  ├─ sync/             ← nube de cuentas: backend.rs, firebase.rs, outbox.rs, engine.rs
      │  └─ util/             ← redact.rs, StreamGuard
      └─ binaries/            ← sidecars ffmpeg y mpv
```

**Regla de ubicación:** lógica de negocio en Rust, presentación en React. Si te
descubres parseando M3U, limpiando títulos o hablando con TMDB desde TypeScript,
estás en el sitio equivocado.

---

## 5. Cómo trabajamos

1. **Plan antes de código.** Para cualquier tarea de más de un fichero, escribe primero
   el plan y espera confirmación. Nada de refactors sorpresa.
2. **Una fase cada vez.** `PROMPTS.md` define el orden. No adelantes trabajo de fases
   futuras "ya que estamos".
3. **Tests donde duele.** Obligatorios en: parser M3U, cliente Xtream, normalizador de
   títulos, parser XMLTV. Con ficheros de ejemplo reales en `tests/fixtures/`. El resto
   de la app no necesita tests unitarios en v1.
4. **Commits pequeños**, en imperativo y en español o inglés, pero consistente.
5. **Cuando termines una fase**, actualiza `docs/DECISIONS.md` con lo que cambió y por
   qué, y marca la fase en `PROMPTS.md`.
6. **Si algo del proveedor real no encaja con la documentación**, para y dilo. Los
   paneles Xtream tienen variaciones entre versiones; no inventes campos.

---

## 6. Convenciones de código

**Rust**
- `thiserror` para errores de dominio, `anyhow` solo en `main.rs`.
- Nada de `unwrap()` ni `expect()` fuera de tests y de la inicialización.
- Toda función pública documentada con una línea. Sin comentarios que narren lo obvio.
- `tracing` para logs, nunca `println!`.

**TypeScript**
- `strict: true`. Nada de `any`. Los tipos de la API se generan desde Rust con `ts-rs`
  en `src/lib/api-types.ts`; **no los edites a mano**.
- Componentes función, sin clases. Sin `React.FC`.
- Un componente por fichero, nombre igual al fichero.
- Estado de servidor en TanStack Query. `useState` solo para estado de UI local.

**UI**
- Se navega con mando a distancia en el futuro, así que **todo elemento interactivo
  debe ser accesible por teclado desde el primer día**: flechas, Enter, Escape/Backspace.
- Tema oscuro únicamente. Sin selector de tema en v1.
- Sin librería de componentes pesada. Componentes propios sobre Tailwind.

---

## 7. Cosas que NO se hacen en este proyecto

- No se transcodifica vídeo salvo como último recurso explícito. Remux sí, transcodificar
  H.265 a H.264 en tiempo real revienta la CPU.
- No se incluye ninguna lista, URL de proveedor ni credencial en el repositorio, ni
  siquiera en ejemplos o tests. Los fixtures son anónimos.
- No se añaden dependencias sin justificarlo en el plan.
- No se implementa DRM, ni descarga masiva de contenido, ni grabación programada en v1.
- No se toca la carpeta `clients/` (Fire TV, iOS, web) hasta la fase 10.
