# Proveedor Xtream falso

Un panel Xtream Codes de mentira para probar la app de punta a punta sin un proveedor
real ni credenciales reales (regla R5). Sin dependencias: Node y ffmpeg en el `PATH`.

```
node tools/proveedor-falso/server.mjs
```

En la app: **Ajustes → Añadir proveedor → Xtream Codes**, URL `http://127.0.0.1:8899`,
usuario `qa`, contraseña `qa`. El puerto se cambia con la variable `PORT`.

## Qué trae

- **16 canales** en 5 categorías, con los casos difíciles de un panel real: prefijos
  (`ES|`, `DEP|`, `XXX|`), feeds duplicados del mismo canal (`La 1 HD` / `La 1 FHD`),
  catch-up en algunos, uno sin EPG y una categoría de adultos.
- **25 películas** con títulos "sucios" (año entre paréntesis o corchetes, calidades,
  idioma) y duplicados en varias calidades que deben agruparse en una sola ficha.
- **4 series** con varias temporadas, que se piden con `get_series_info` al abrir la
  ficha.
- **Guía XMLTV** de ayer a pasado mañana, servida en gzip sin `Content-Encoding`, como
  hacen muchos paneles.
- **Vídeo de verdad** generado con ffmpeg la primera vez (se cachea en
  `%TEMP%\vibe-proveedor-falso`):
  - directo en MPEG-TS con AAC (los canales de deportes, con AC3), enviado al ritmo de
    reproducción como un canal de verdad;
  - "Reloj (pruebas de pausa)": media hora de directo con un contador de segundos en
    pantalla y audio MP2, para comprobar a ojo que pausar y rebobinar el directo retoma
    en el segundo exacto. Ojo: el proveedor manda al ritmo medio del fichero, no al de
    sus marcas de tiempo, así que tras muchos minutos va algo por detrás del reloj;
  - películas en MP4 (nivel 1); en MKV con dos pistas de audio y dos de subtítulos SRT
    (nivel 2). En el MKV, `testsrc` pinta un contador de segundos y cada frase dice en qué
    segundo debe verse ("Segundo 30"), así que una captura enseña si van a tiempo;
  - una en H.265/HEVC ("Planeta Tierra II"), que una WebView sin HEVC manda a mpv
    (nivel 3), y una en H.264 de 10 bits ("Akira"), que la WebView2 sí decodifica.
  - "Prueba de fuente lenta" en dos fuentes: la de 1080p llega a menos de la mitad de lo
    que hace falta para verla (se para enseguida) y la de 720p va bien. El reproductor
    tiene que pasar solo a la segunda, con el aviso y «Volver» (F17).
  - en la guía de los canales de deportes, partidos al estilo Movistar Plus (F17): un
    Real Madrid - Barcelona en juego en dos canales (sale en la portada de Inicio), un
    tenis dentro de unas horas, un Sevilla - Betis mañana, y un resumen y una repetición de
    otra temporada que no deben salir en Deportes.

En la consola avisa si en algún momento hay más de una conexión de directo abierta a la
vez: la app nunca debería abrir dos (regla R1).

## Simular un proveedor de verdad

- `VIBE_FALSO_BACHES=1`: en el directo, cada 10 s deja de mandar 1,5 s y luego manda lo
  atrasado de golpe, como un panel tras una VPN. Para medir los parones del reproductor.
- `VIBE_FALSO_RAFAGA=30`: la ráfaga inicial del directo dura 30 s en vez de 3.
