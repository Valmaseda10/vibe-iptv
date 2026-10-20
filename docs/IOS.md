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
| Pausar y rebobinar el directo | sí (F13) | no: depende de ffmpeg |
| Subtítulos y pistas de audio | selector propio | los menús de VLC (incluye subtítulos incrustados) |
| Zapping | flechas, números | deslizar arriba/abajo en el reproductor |
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

---

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
