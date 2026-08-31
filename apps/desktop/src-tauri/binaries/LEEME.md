# Sidecars

Esta carpeta necesita tres ficheros antes de generar el instalador (`npm run tauri
build`). No están en el repositorio (~550 MB en total, no tiene sentido versionarlos) —
descárgalos y colócalos aquí con estos nombres exactos:

| Fichero | De dónde | Versión usada al escribir esto |
|---|---|---|
| `ffmpeg-x86_64-pc-windows-msvc.exe` | [gyan.dev, build "full"](https://www.gyan.dev/ffmpeg/builds/) (o `winget install Gyan.FFmpeg`) | 8.1 |
| `ffprobe-x86_64-pc-windows-msvc.exe` | El mismo paquete que ffmpeg, viene junto | 8.1 |
| `mpv-x86_64-pc-windows-msvc.exe` | [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases), build `mpv-x86_64-<fecha>-git-<hash>.7z` (no el `-dev`, no el `-v3`) | v0.41.0 |

El nombre del fichero importa: Tauri asocia cada sufijo de triple de compilación
(`x86_64-pc-windows-msvc` en Windows de 64 bits) con la entrada correspondiente de
`bundle.externalBin` en `tauri.conf.json`, y al empaquetar copia el binario correcto
junto al ejecutable principal con el sufijo retirado (queda como `ffmpeg.exe`, etc. —
ver el comentario de `media::resolve_binary` en el núcleo).

`cargo check`/`test`/`tauri dev` funcionan sin estos ficheros: el `bundle.active` de
`tauri.conf.json` está en `false` a propósito (si se pone en `true` con
`externalBin` configurado, `cargo` deja de compilar en cuanto falta uno de estos
ficheros — hasta `cargo check` sin intención de empaquetar nada, algo descubierto
construyendo esto). El empaquetado de verdad usa una config aparte que sí los exige:

```
npm run build:installer
```

(`tauri build --config src-tauri/tauri.bundle.conf.json`, ver ese fichero). Solo hace
falta rellenar esta carpeta para generar el instalador — es el criterio de aceptación
de F10 ("instalo el .msi en un PC limpio y funciona").

## Nota de licencias

ffmpeg ("full build" de gyan.dev) y mpv se distribuyen bajo GPL. Empaquetarlos como
sidecars (procesos aparte, invocados por tubería/IPC, nunca enlazados con el binario de
la app) es "agregación", no obliga a que el resto de VIBE IPTV sea GPL — pero sí exige
ofrecer o enlazar el código fuente de **esos binarios exactos** en cuanto el instalador
se comparta con alguien más, no solo para uso personal en esta máquina. Antes de
distribuir el instalador fuera de aquí:
- Cambiar a un build **LGPL** de ffmpeg (basta: la app nunca recodifica vídeo, solo
  copia/remuxea y como mucho transcodifica audio a AAC con el códec nativo de ffmpeg,
  nada de lo que usa esta app necesita los codificadores GPL como libx264/libx265).
- Enlazar el código fuente exacto de la build de mpv usada (mpv es GPL salvo que se
  compile explícitamente en modo LGPL, y los builds de `shinchiro` normales no lo son).

Hash de los binarios usados para esta primera build (para poder confirmar más adelante
que el instalador se generó con lo que dice esta tabla):

```
d1e2a156261ecc675081943197a85f08f2868784a0af499171ede89353edad31  ffmpeg-x86_64-pc-windows-msvc.exe
70872c3ffbc43d0b2c570f9837f54d6e9a832f4ca25463e9735b6a3ec0621478  ffprobe-x86_64-pc-windows-msvc.exe
a444e80fa3985c83bc3fe76e32b76b2ee396c1a5b6a9fd8d2eecc10780054c41  mpv-x86_64-pc-windows-msvc.exe
```
