#!/usr/bin/env bash
# Prueba del reproductor nativo en un iPhone simulado, con toques de verdad (idb).
# La lanza `.github/workflows/ios.yml` con `qa_player`. Deja capturas y lo que ve la
# accesibilidad en cada paso en $OUT, y falla si algo no responde como debe.
#
# Uso: player-qa.sh <udid> <ruta.app> <dir-salida>
set -uo pipefail
UDID="$1"
APP="$2"
OUT="$3"
mkdir -p "$OUT"
HERE="$(cd "$(dirname "$0")" && pwd)"
fail=0

shot() { xcrun simctl io "$UDID" screenshot "$OUT/$1.png" >/dev/null 2>&1; }
a11y() { idb ui describe-all --udid "$UDID" --json > "$OUT/$1.json" 2>/dev/null || true; }
# Centro del elemento cuya etiqueta de accesibilidad es $1 (o vacío si no está).
center_of() {
  python3 - "$OUT/$2.json" "$1" <<'PY'
import json, sys
try:
    items = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
# El reproductor pone el iPhone en horizontal: idb da los marcos en coordenadas de la
# interfaz pero toca en las de la pantalla en vertical. Se devuelven ya en las de tocar.
root = (items[0].get("frame") or {}) if items else {}
landscape = root.get("width", 0) > root.get("height", 0)
portrait_w = root.get("height", 0)
for it in items:
    if (it.get("AXLabel") or "") == sys.argv[2]:
        f = it.get("frame") or {}
        x = f.get('x', 0) + f.get('width', 0) / 2
        y = f.get('y', 0) + f.get('height', 0) / 2
        if landscape:
            x, y = portrait_w - y, x
        print(f"{x:.0f} {y:.0f}")
        break
PY
}
visible() { [ -n "$(center_of "$1" "$2")" ]; }
# Espera hasta $2 segundos a que aparezca la etiqueta $1; deja el último volcado en $3.
wait_visible() {
  for _ in $(seq 1 "$2"); do
    a11y "$3"
    if visible "$1" "$3"; then return 0; fi
    sleep 1
  done
  return 1
}
check() {
  if eval "$2"; then echo "ok   $1"; else echo "MAL  $1"; fail=1; fi
}

# Servidor del vídeo de prueba (con Range, como un proveedor).
npx --yes http-server "$HERE" -p 8765 -s >/dev/null 2>&1 &
for _ in $(seq 1 60); do curl -sf -o /dev/null -r 0-10 http://127.0.0.1:8765/prueba.mkv && break; sleep 1; done

bundle=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$APP/Info.plist")
xcrun simctl install "$UDID" "$APP"
SIMCTL_CHILD_VIBE_QA_PLAYER_URL="http://127.0.0.1:8765/prueba.mkv" SIMCTL_CHILD_RUST_BACKTRACE=1 \
  xcrun simctl launch "$UDID" "$bundle" >/dev/null
# Estado exacto del reproductor (VibePlayer.swift lo escribe en modo prueba): idb a veces
# devuelve la pantalla vacía justo en una transición.
state_field() {
  local data file
  data=$(xcrun simctl get_app_container "$UDID" "$bundle" data 2>/dev/null)
  file=$(find "$data" -name qa-player.json 2>/dev/null | head -1)
  [ -n "$file" ] && python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2], ''))" "$file" "$1" 2>/dev/null
}
# 1. Arranca y enseña los controles (el modo de prueba abre el reproductor a los 4 s).
ok1=1
# Primero el estado del reproductor: si idb tarda en responder, los controles ya se habían
# escondido (a los 4 s) cuando llegaba su respuesta.
for _ in $(seq 1 80); do
  if [ "$(state_field controls)" = "True" ]; then ok1=0; break; fi
  sleep 0.3
done
if [ $ok1 -ne 0 ]; then a11y 01-arranque; visible "Cerrar" 01-arranque && ok1=0; fi
a11y 01-arranque
shot 01-arranque
check "los controles salen al empezar" '[ $ok1 -eq 0 ]'

# 2. Los controles se esconden solos a los 4 s, y solo lo hacen con el vídeo en marcha:
#    que desaparezcan prueba a la vez que se reproduce.
hidden=1
for _ in $(seq 1 30); do
  a11y 02-sin-controles
  if ! visible "Cerrar" 02-sin-controles; then hidden=0; break; fi
  sleep 1
done
shot 02-sin-controles
check "el vídeo se reproduce y los controles se esconden solos" '[ $hidden -eq 0 ]'

# 3. Un toque en el centro los vuelve a enseñar.
read -r W H < <(python3 -c "import json;d=json.load(open('$OUT/01-arranque.json'));f=d[0]['frame'];print(int(f['width']),int(f['height']))" 2>/dev/null || echo "400 800")
for _ in 1 2 3; do
  [ "$(state_field controls)" = "True" ] && break
  idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5
done
shot 03-tras-tocar; a11y 03-tras-tocar
check "tocar la pantalla enseña los controles" 'visible "Cerrar" 03-tras-tocar || [ "$(state_field controls)" = "True" ]'

# 4. +30 s no saca del reproductor.
xy=$(center_of "Adelante 30 segundos" 03-tras-tocar)
if [ -n "$xy" ]; then idb ui tap --udid "$UDID" $xy; fi
sleep 4
# idb a veces devuelve el árbol vacío justo en una transición: se reintenta.
for _ in 1 2 3; do
  idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5
  shot 04-tras-avanzar; a11y 04-tras-avanzar
  visible "Cerrar" 04-tras-avanzar && break
done
check "avanzar 30 s sigue en el reproductor" 'visible "Cerrar" 04-tras-avanzar'

# 5. Pausa y reanuda.
xy=$(center_of "Reproducir o pausar" 04-tras-avanzar)
if [ -n "$xy" ]; then idb ui tap --udid "$UDID" $xy; sleep 1; shot 05-en-pausa; idb ui tap --udid "$UDID" $xy; fi

# 6. Cerrar vuelve a la app.
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5; a11y 06-antes-de-cerrar
xy=$(center_of "Cerrar" 06-antes-de-cerrar)
if [ -n "$xy" ]; then idb ui tap --udid "$UDID" $xy; fi
sleep 2
shot 06-cerrado; a11y 06-cerrado
check "cerrar sale del reproductor" '! visible "Cerrar" 06-cerrado'

# 7. Directo: los mismos controles con la etiqueta de directo, y tocar los enseña.
xcrun simctl terminate "$UDID" "$bundle" >/dev/null 2>&1 || true
SIMCTL_CHILD_VIBE_QA_PLAYER_URL="http://127.0.0.1:8765/prueba.mkv" SIMCTL_CHILD_VIBE_QA_PLAYER_LIVE=1   xcrun simctl launch "$UDID" "$bundle" >/dev/null
wait_visible "Cerrar" 25 07-directo; ok7=$?
shot 07-directo
check "directo: los controles salen al empezar" '[ $ok7 -eq 0 ]'
for _ in $(seq 1 15); do [ "$(state_field controls)" = "True" ] || break; sleep 1; done
for _ in 1 2 3; do
  [ "$(state_field controls)" = "True" ] && break
  idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5
done
shot 08-directo-toque; a11y 08-directo-toque
check "directo: tocar enseña los controles" 'visible "Cerrar" 08-directo-toque || [ "$(state_field controls)" = "True" ]'

xcrun simctl spawn "$UDID" log show --last 3m --predicate "process CONTAINS 'VIBE'" > "$OUT/log.txt" 2>/dev/null || true
if grep -q "panicked at" "$OUT/log.txt"; then echo "MAL  el núcleo entró en pánico"; grep -A3 "panicked at" "$OUT/log.txt" | head; fail=1; fi
exit $fail
