# VIBE IPTV en iPhone y iPad (F15)

La app de iPhone y iPad es la misma app del PC (Tauri 2) compilada para iOS: misma
interfaz, mismo núcleo en Rust, misma base de datos SQLite. **Funciona sin el PC**: habla
directamente con el proveedor, guarda sus credenciales en el llavero del iPhone y comparte
perfiles, favoritos y "Seguir viendo" con el PC a través de Firebase (ver `CLOUD-SYNC.md`).

Se distribuye **sin la cuenta de desarrollador de Apple**: se compila sin firmar en un
Mac de GitHub y se instala desde Windows con Sideloadly, que la firma con un Apple ID
normal. La firma gratuita caduca a los **7 días**.

---

## Diferencias con el PC

| | PC | iPhone / iPad |
|---|---|---|
| Reproductor | `<video>` + remux ffmpeg + mpv | VLC (MobileVLCKit 3.7.4) en una pantalla nativa |
| Lo que entra al reproductor | según la cascada de 3 niveles | los bytes del proveedor tal cual (`?passthrough=true`) |
| Pausar y rebobinar el directo | sí, con ffmpeg (F13) | sí, sin ffmpeg: el núcleo graba el directo tal cual y VLC lee de la grabación (`server::live_buffer`, unos 15 min) |
| Subtítulos y pistas de audio | selector propio | los menús de VLC (incluye subtítulos incrustados) |
| Zapping | flechas, números | deslizar arriba/abajo en el reproductor |
| Salir del reproductor | Escape | botón ✕, o deslizar hacia abajo en películas y series |
| Llenar la pantalla | — | doble toque o el botón de las flechas |
| Navegación | barra lateral | pestañas abajo en iPhone; barra lateral en iPad |

Lo demás (catálogo, búsqueda, guía, perfiles, control parental, sincronización con TMDB y
EPG) es el mismo código.

---

## Instalar o renovar la app

Una sola vez en el PC: **Sideloadly** (sideloadly.io) e **iTunes e iCloud descargados de
apple.com** (las versiones de la Microsoft Store no le valen).

1. GitHub → repositorio → **Actions** → **iOS** → **Run workflow**. Tarda unos 20-30 min.
2. Al terminar, en el resumen de la ejecución, descargar el artefacto **VIBE-IPTV-ipa**
   (un .zip con `VIBE-IPTV.ipa` dentro) y descomprimirlo.
3. Conectar el iPhone por cable al PC y desbloquearlo ("Confiar en este ordenador").
4. Sideloadly: arrastrar `VIBE-IPTV.ipa`, escribir el Apple ID y pulsar **Start**. Pide
   la contraseña del Apple ID (y el código de verificación la primera vez).
5. Solo la primera vez, en el iPhone:
   - Ajustes → General → VPN y gestión de dispositivos → confiar en el perfil del Apple ID.
   - Ajustes → Privacidad y seguridad → **Modo de desarrollador** → activar y reiniciar.

**Renovar cada 7 días**: repetir el paso 4 con el mismo .ipa (no hace falta compilar de
nuevo). Los datos de la app se conservan. Sideloadly también puede renovarla solo si el
iPhone y el PC están en la misma Wi-Fi ("Automatic refresh" en sus opciones).

Límites de un Apple ID gratuito: 3 apps instaladas así a la vez y 10 identificadores de
app nuevos por semana.

### Sin renovar a mano: SideStore (recomendado)

No hay forma gratuita de firmar una app para siempre en iOS 26: Apple mantiene los 7 días
para cuentas gratuitas, TrollStore no funciona desde iOS 17.0.1 y no hay jailbreak
práctico. Las tiendas alternativas de la UE (AltStore PAL) y la descarga web exigen la
cuenta de desarrollador de pago (investigado el 2026-10-20). Lo más cercano es que la
renovación se haga sola:

- **SideStore** renueva la app en el propio iPhone, sin PC (el PC solo hace falta la
  primera vez). Hay guías probadas en iOS 26.2–26.5 con Windows 11.
  1. En el PC: iTunes (de apple.com) e **iloader**.
  2. En el iPhone: código de desbloqueo, **LocalDevVPN** (App Store) conectado.
  3. Por cable, iloader instala SideStore con el Apple ID y genera el fichero de
     emparejamiento. Activar el Modo de desarrollador y confiar en el perfil.
  4. Desde SideStore, instalar `VIBE-IPTV.ipa` (ocupa 2 de los 3 huecos gratuitos).
  5. App **Atajos** → Automatización diaria de madrugada → «Refresh Apps» de SideStore,
     ejecutar sin preguntar.
  - Tras reiniciar el iPhone hay que volver a conectar LocalDevVPN. Si el atajo falla,
    abrir SideStore y pulsar «Refresh All» antes de que pasen 7 días.
- **Plan B:** la renovación automática de Sideloadly, con el PC encendido en la misma Wi-Fi
  al menos una vez por semana.

---

## Aspecto (rediseño «Liquid Glass»)

Solo en iPhone/iPad; el PC se queda como estaba. `main.tsx` pone la clase `ios` en `<html>`
y `globals.css` define la variante `ios:` y el material: `glass` (cristal con desenfoque),
`glass-clear` (sobre imagen o vídeo) y `glass-lite` (mismo aspecto sin desenfoque, para lo
que se repite). Reglas:

- **El cristal solo flota:** barra de pestañas, cabecera compacta, botones sobre el fondo de
  las fichas, hojas y controles del reproductor. Nunca en portadas, filas ni celdas: cada
  `backdrop-filter` se recompone en cada fotograma de desplazamiento.
- **Nunca cristal sobre cristal:** lo de dentro de una hoja va sin cristal propio.
- **Fondo negro puro** y letra del sistema (SF).
- **«Reducir transparencia»** de iOS (comando `reduce_transparency`, leído en Swift): el
  cristal pasa a casi opaco. En el reproductor, el cristal de iOS 26 (`UIGlassEffect`) y,
  antes, el desenfoque oscuro.
- **Piezas:** `MobileTabBar` (cápsula flotante con la «gota» de la pestaña activa),
  `GlassSheet` (hoja que se cierra arrastrando), `LargeTitle` (título grande que se
  compacta), `FloatingBackButton`, Buscar abajo en el iPhone (`lib/viewport.ts` sigue al
  teclado), barra lateral de cristal en el iPad.

## Lo que hay que saber de iOS

- **Segundo plano.** iOS puede dejar inservible el socket del servidor local de una app
  suspendida. Al volver a primer plano, la interfaz llama a `ensure_server`, que
  comprueba `/api/health` y, si no responde, vuelve a abrir el servidor (en el mismo
  puerto si puede). Una petición que falle por red también lo intenta y se repite una vez.
- **Diálogos del navegador.** `window.confirm`/`alert`/`prompt` no existen en el WebView de
  Tauri en iOS (devuelven "no" sin enseñar nada). La interfaz usa confirmaciones propias.
- **Sin ratón.** Nada puede depender de `hover` para aparecer, y los botones pequeños
  amplían su zona táctil.

## Pruebas

- **Interfaz:** Playwright con WebKit (el motor del iPhone) a tamaño iPhone, iPad e iPhone
  en horizontal, contra la app de escritorio en marcha. Chromium no veía cosas que WebKit
  sí (tarjetas que se ensanchaban, rejillas que se arrastraban de lado).
- **Reproductor nativo:** el flujo de GitHub con `qa_player` compila para el simulador,
  abre el reproductor con un vídeo sintético (`ios/qa/prueba.mkv`) y lo maneja con toques
  reales (idb): controles al empezar, se esconden solos, tocar los enseña, +30 s, cerrar,
  y lo mismo en modo directo. Deja capturas y el volcado de accesibilidad de cada paso.
- **De punta a punta:** con `qa_e2e`, la app de depuración arranca en un iPhone y un iPad
  simulados con un perfil y el proveedor falso (`tools/proveedor-falso`, en el propio Mac)
  y `ios/qa/e2e-qa.sh` la recorre con toques: perfil, pestañas, menú "Más", mantener
  pulsada una portada, ficha, película (+30 s, cerrar) y directo (pausa, "Ir al directo",
  zapping deslizando), comprobando que la barra de pestañas llega abajo y que nunca hay
  más de una conexión con el proveedor (R1). Son ~70 comprobaciones por aparato, en
  secciones (perfil, navegación, Mi lista, película, continuar viendo, serie, directo,
  Buscar, Ajustes, perfiles, segundo plano). Cada sección mira antes si la app se cerró
  sola (lo apunta como fallo y la vuelve a abrir) o si quedó el reproductor abierto (lo
  cierra), para que un fallo no arrastre al resto. Los informes de fallo del sistema se
  guardan con las capturas.
- **Estado exacto del reproductor.** En modo de pruebas, `VibePlayer.swift` escribe cada
  medio segundo `qa-player.json` (posición, pausa, retraso del directo, título, controles,
  lista y tamaño de la interfaz): la prueba comprueba con eso, no con lo que idb crea ver,
  y gira sus toques cuando el iPhone está en horizontal (idb toca en coordenadas de la
  pantalla en vertical). En la prueba de punta a punta los controles tardan 15 s en
  esconderse en vez de 4: cada consulta a idb tarda 2 o 3 s.
- **Ver dentro del WebView.** idb solo ve lo nativo (el reproductor): los botones de la
  página no salen en su árbol de accesibilidad. En modo de pruebas (`VIBE_QA_E2E`, solo en
  depuración) la página escribe cada medio segundo `qa-ui.json` en los datos de la app
  con lo que se ve y dónde (`src/lib/qaSnapshot.ts`), y el script lo junta con lo de idb.
- **Qué saber qué versión hay instalada:** Ajustes de la app ("Compilación ios-N") y
  Ajustes del iPhone → General → Almacenamiento → VIBE IPTV (versión 0.1.0.N).

## Cómo está montado

```
apps/desktop/src-tauri/
├─ src/lib.rs                ← `run()`: el mismo arranque para PC y móvil
├─ src/native_player.rs      ← puente Rust ↔ Swift (dos funciones C con JSON)
├─ ios/Sources/VibePlayer.swift  ← pantalla del reproductor (VLC)
├─ ios/project.yml           ← plantilla de Xcode: la de tauri-cli + Swift + VLC
├─ tauri.ios.conf.json       ← identificador, iOS mínimo (17) y plantilla
├─ Info.ios.plist            ← permite http a 127.0.0.1 y a los logos del proveedor
└─ icons/ios/                ← iconos (generados con `tauri icon`)
.github/workflows/ios.yml    ← compila el .ipa sin firmar (y opcionalmente lo abre en un simulador)
```

- **Por qué no un plugin de Tauri.** MobileVLCKit es un framework dinámico que Xcode
  tiene que incrustar en la app; un paquete Swift de plugin lo compila `swift-rs` fuera
  de Xcode y no lo enlaza. El Swift del reproductor vive en el propio destino de la app,
  que sí lo enlaza, y el núcleo lo llama por `vibe_player_present`.
- **R1 y R5 siguen igual.** VLC solo ve `http://127.0.0.1:<puerto>/stream/...`: las
  credenciales no salen del núcleo y el `StreamGuard` sigue repartiendo la única
  conexión. Con el PC encendido a la vez son dos dispositivos: si los dos reproducen, el
  proveedor cortará uno (la cuenta admite una conexión).
- **Minutos de Mac.** El plan gratuito de GitHub da unos 200 minutos de Mac al mes en
  repositorios privados. Por eso la compilación solo se lanza a mano y guarda en caché
  Rust y VLC.
