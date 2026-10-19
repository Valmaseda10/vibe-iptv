# Arquitectura — VIBE IPTV

## Visión general

```
┌──────────────────────── Aplicación Tauri (un único proceso) ─────────────────────┐
│                                                                                  │
│  WebView (React)                        Núcleo Rust                              │
│  ┌───────────────────┐                  ┌────────────────────────────────────┐   │
│  │ Rutas y pantallas │◄──── HTTP ──────►│ axum @ 127.0.0.1:<puerto efímero>  │   │
│  │ TanStack Query    │   /api/*         │  ├─ /api/*     catálogo, EPG, estado│   │
│  │ <video> + hls.js  │◄──── HTTP ──────►│  ├─ /stream/*  proxy y remux        │   │
│  └───────────────────┘   /stream/*      │  └─ /img/*     caché de imágenes    │   │
│           ▲                             │                                     │   │
│           │ comandos Tauri              │  Sincronizador   StreamGuard        │   │
│           └────────────────────────────►│  SQLite + FTS5   keyring            │   │
│                                          └───────┬────────────────────────────┘   │
│                                                  │                                │
│                              sidecars ───────────┼──────────►  ffmpeg   mpv       │
└──────────────────────────────────────────────────┼────────────────────────────────┘
                                                   ▼
                                    Proveedor IPTV        TMDB
```

**Por qué un servidor HTTP local en vez de solo comandos Tauri:** el elemento `<video>`
necesita una URL. Un comando Tauri no puede servir un flujo de vídeo continuo. Además,
esa misma superficie HTTP es lo que en la fase 10 se abre a la LAN para que el cliente
de Fire TV la consuma sin reescribir nada del núcleo. Los comandos Tauri se reservan
para operaciones de ventana, diálogos de fichero y acceso al keychain.

El puerto es efímero y se comunica al frontend en el arranque mediante un comando
Tauri. Nada de puertos fijos hardcodeados.

---

## Capa de fuentes

Un trait común aísla las diferencias entre proveedores:

```rust
#[async_trait]
pub trait Source {
    async fn account_info(&self) -> Result<AccountInfo>;      // caducidad, max_connections
    async fn categories(&self, kind: MediaKind) -> Result<Vec<RawCategory>>;
    async fn live_channels(&self) -> Result<Vec<RawChannel>>;
    async fn movies(&self) -> Result<Vec<RawMovie>>;
    async fn series(&self) -> Result<Vec<RawSeries>>;
    async fn series_detail(&self, id: &str) -> Result<RawSeriesDetail>;  // temporadas y episodios
    async fn epg_xmltv(&self) -> Result<Option<XmltvStream>>;
    fn stream_url(&self, item: &StreamRef) -> Result<Url>;
}
```

Implementaciones: `XtreamSource` y `M3uSource`. `M3uSource` deja `series_detail` sin
implementar de verdad y agrupa episodios a partir de patrones `S01E02` en los títulos.

**Parseo M3U:** en streaming, línea a línea, sin cargar el fichero en memoria. Los
atributos de `#EXTINF` que importan son `tvg-id`, `tvg-name`, `tvg-logo`,
`group-title`. La clasificación entre directo, película y serie se hace por la ruta de
la URL (`/live/`, `/movie/`, `/series/`) y, si no la hay, por heurística sobre
`group-title`.

---

## Cascada de reproducción

Tres niveles. Se elige el más barato que funcione, y se baja de nivel si falla.

### Nivel 1 — Directo al `<video>`
Aplicable cuando el stream es HLS (`.m3u8`) o un MP4 con H.264/AAC. HLS se maneja con
hls.js, salvo en Safari que lo hace nativo. Coste de CPU: cero. Latencia de arranque:
mínima. Aun así el tráfico pasa por `/stream/*` para no exponer credenciales al WebView
y para controlar cabeceras y `User-Agent`.

### Nivel 2 — Remux a fMP4 en el proxy local
Aplicable a MPEG-TS y a MKV con códecs compatibles. `ffprobe` inspecciona el stream
una vez y el resultado se cachea por ítem. Después, ffmpeg reempaqueta sin
recodificar vídeo:

```
ffmpeg -hide_banner -loglevel error \
  -probesize <1M directo | 2M VOD> -analyzeduration <1M directo | 2M VOD> \
  -i <url> \
  -c:v copy -c:a <copy [-bsf:a aac_adtstoasc si es AAC] | aac -b:a 192k> \
  -f mp4 -movflags frag_keyframe+empty_moov+default_base_moof+delay_moov \
  -frag_duration 500000 \
  pipe:1
```

El audio se recodifica solo si es AC3, E-AC3 o DTS **y el cliente no lo decodifica**:
el reproductor manda `?audio_passthrough=ac3,eac3` con lo que su WebView reproduce
(`canPlayType`). La WebView2 de Windows reproduce AC-3/E-AC-3, así que ahí el Dolby se
copia tal cual y se conserva el 5.1; DTS se sigue convirtiendo. El vídeo **nunca** se
recodifica en este nivel.

Tres lecciones medidas (ver `DECISIONS.md`, 2026-10-16):
- **Sin `-fflags nobuffer`.** Descarta lo leído mientras se analiza la entrada, que es
  justo la ráfaga de búfer que mandan los paneles al conectar: el directo tardaba 4,3 s
  en dar el primer fragmento (0,1 s sin él), y el VOD perdía sus primeros segundos.
- **`aac_adtstoasc` al copiar AAC.** El AAC de un MPEG-TS va en ADTS; ffmpeg 8 ya no
  añade el filtro solo y aborta el remux tras escribir solo la cabecera.
- **Fragmentos de 0,5 s** además de en cada fotograma clave: con GOP largos, esperar al
  siguiente clave retrasaba varios segundos el primer fotograma.

Para directo, cada cambio de canal mata el proceso ffmpeg anterior antes de lanzar el
nuevo. Un proceso vivo por sesión, sin excepciones.

**Subtítulos incrustados (F12).** Si el cliente pide `?subtitle_track=N`, el **mismo**
proceso añade una segunda salida, sin abrir otra conexión con el proveedor (R1):

```
  ... pipe:1 \
  -map 0:s:N -c:s webvtt -ss 0 -flush_packets 1 -f webvtt <temporal>.vtt
```

La salida principal lleva entonces `-map` explícitos (vídeo y la pista de audio
elegida, o la primera) y siempre `-sn`, para que ffmpeg no meta por su cuenta los
subtítulos de texto como `mov_text` dentro del fMP4. La API lee el `.vtt` mientras se
escribe y el reproductor añade las frases al `<video>` según llegan
(`/api/playback/subtitles`). Solo los de texto: los de imagen (PGS, DVD) necesitarían
OCR o quemarlos en el vídeo, y eso es recodificar. Ver los detalles medidos
(`-ss 0`, frase en curso) en `DECISIONS.md`, F12.

**Pausar y rebobinar el directo (F13).** En los canales en directo, el mismo proceso
añade otra salida: la grabación en trozos HLS fMP4 de 2 s, con una ventana deslizante de
30 min, en una carpeta temporal que se borra al cambiar de canal. Una sola conexión con
el proveedor (R1).

```
  ... pipe:1 \
  -map 0:v:0 -map 0:a:N -c:v copy -c:a <copy | aac si es MP2/MP3> \
  -f hls -hls_time 2 -hls_list_size 900 -hls_flags delete_segments+independent_segments+temp_file \
  -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4 <carpeta>/live.m3u8
```

El reproductor ve el directo por `pipe:1` (el zapping no cambia) y, al pausar o
rebobinar, pasa a la grabación con hls.js. La sesión ya no muere con la respuesta HTTP
(en pausa no se lee el directo): vive mientras se lea o lleguen señales de vida, y ffmpeg
se lee siempre aunque nadie consuma el directo, porque si se bloquease se pararía también
la grabación. Detalle en `server::timeshift` y `DECISIONS.md` (F13).

### Nivel 3 — mpv como respaldo
Para todo lo demás: H.265 sin soporte de hardware, VC-1, contenedores raros, o cuando
el nivel 2 falla dos veces seguidas. En v1, mpv se lanza en su propia ventana y se
controla por IPC (`--input-ipc-server`). No es bonito, pero reproduce todo.

Empotrar mpv dentro de la ventana de la aplicación con superposición de interfaz es
trabajo de una fase posterior, no de v1. Está anotado en `DECISIONS.md` como deuda
técnica conocida.

**Diagnóstico:** la app registra en el historial qué nivel se usó para cada ítem. Si el
nivel 3 aparece a menudo, es señal de que hay que revisar la detección de códecs.

---

## StreamGuard

Un único semáforo de permiso global en el núcleo:

```rust
pub struct StreamGuard { inner: Mutex<Option<ActiveSession>> }
```

- `acquire(item)` cierra la sesión anterior, espera a que el proceso hijo muera de
  verdad (no solo a enviar la señal) y entonces devuelve el permiso.
- El sincronizador comprueba `is_busy()` antes de cada lote y se detiene si hay
  reproducción.
- Al cerrar el reproductor o la ventana, se libera siempre. Hay un `Drop` que lo
  garantiza también en caso de pánico.

Motivo: los paneles Xtream cuentan conexiones por sesión abierta, y una conexión
huérfana bloquea la cuenta durante minutos.

---

## Sincronización

```
1. account_info()          → validar credenciales, leer max_connections
2. categories()            → upsert de categorías
3. live_channels()         → upsert por lotes de 500 en una transacción
4. movies() y series()     → ídem
5. cola de normalización   → limpieza de títulos, agrupación por calidad
6. cola de TMDB            → 5 peticiones concurrentes como máximo, con reintentos
7. cola de imágenes        → descarga a la caché de disco
8. EPG                     → descarga XMLTV, parseo en streaming, inserción por lotes
```

Los pasos 5 a 8 son de segundo plano y no bloquean la interfaz: el catálogo se puede
usar en cuanto termina el paso 4, con portadas apareciendo progresivamente.

El EPG merece cuidado aparte: un XMLTV comprimido de 30 MB se convierte en cientos de
miles de programas. Se parsea con `quick-xml` en streaming, se descartan los programas
anteriores a ayer y posteriores a siete días, y se insertan por lotes en una
transacción con índice sobre `(channel_id, start)`.

---

## Decisiones tomadas y sus alternativas

| Decisión | Alternativa descartada | Motivo |
|---|---|---|
| Tauri 2 | Electron | 15 MB frente a 200 MB, y menos RAM en reposo |
| Núcleo en Rust | Núcleo en Node como sidecar | Un solo binario, sin runtime que empaquetar |
| SQLite | Ficheros JSON | 50.000 ítems con búsqueda y filtros necesitan índices |
| HTTP local | Solo comandos Tauri | `<video>` necesita URL, y prepara el cliente de TV |
| Remux antes que mpv | mpv desde el principio | Permite superponer la interfaz HTML sobre el vídeo |
| TMDB | Metadatos del proveedor | Los proveedores dan portadas incompletas o rotas |

---

## Camino hacia las demás plataformas

La fase 10 abre `0.0.0.0` con emparejamiento por código y token. A partir de ahí:

- **Fire TV**: cliente nativo que consume `/api` cuando el PC está encendido, con caché
  local del catálogo para poder navegar aunque el PC esté apagado. La reproducción
  puede ir directa al proveedor, saltándose el PC, porque ExoPlayer sí reproduce TS.
  Ojo con el StreamGuard: el permiso pasa a ser distribuido y lo concede el PC.
- **Web**: la misma API, servida por la app de escritorio. Limitada por códecs.
- **iOS**: SwiftUI con VLCKit contra la misma API. Sin publicación en App Store.
