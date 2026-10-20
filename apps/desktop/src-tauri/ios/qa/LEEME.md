# Prueba del reproductor en el simulador

`prueba.mkv` es un vídeo sintético (testsrc2 + un tono, 2 min, generado con ffmpeg): nada de ningún proveedor. `player-qa.sh` lo sirve en local y maneja el reproductor con toques reales (idb). Lo lanza `.github/workflows/ios.yml` con la opción `qa_player`.
