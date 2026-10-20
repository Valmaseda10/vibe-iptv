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
for it in items:
    if (it.get("AXLabel") or "") == sys.argv[2]:
        f = it.get("frame") or {}
        print(f"{f.get('x', 0) + f.get('width', 0) / 2:.0f} {f.get('y', 0) + f.get('height', 0) / 2:.0f}")
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
sleep 3

bundle=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$APP/Info.plist")
xcrun simctl install "$UDID" "$APP"
SIMCTL_CHILD_VIBE_QA_PLAYER_URL="http://127.0.0.1:8765/prueba.mkv" SIMCTL_CHILD_RUST_BACKTRACE=1 \
  xcrun simctl launch "$UDID" "$bundle" >/dev/null
# 1. Arranca y enseña los controles (el modo de prueba abre el reproductor a los 4 s).
wait_visible "Cerrar" 25 01-arranque; ok1=$?
shot 01-arranque
check "los controles salen al empezar" '[ $ok1 -eq 0 ]'

# 2. A los 4 s sin tocar, se esconden.
sleep 7
shot 02-sin-controles; a11y 02-sin-controles
check "sin tocar, los controles se esconden" '! visible "Cerrar" 02-sin-controles' 

# 3. Un toque en el centro los vuelve a enseñar.
read -r W H < <(python3 -c "import json;d=json.load(open('$OUT/01-arranque.json'));f=d[0]['frame'];print(int(f['width']),int(f['height']))" 2>/dev/null || echo "400 800")
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2))
sleep 1.5
shot 03-tras-tocar; a11y 03-tras-tocar
check "tocar la pantalla enseña los controles" 'visible "Cerrar" 03-tras-tocar'

# 4. +30 s no saca del reproductor.
xy=$(center_of "Adelante 30 segundos" 03-tras-tocar)
if [ -n "$xy" ]; then idb ui tap --udid "$UDID" $xy; fi
sleep 4
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5
shot 04-tras-avanzar; a11y 04-tras-avanzar
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
sleep 7
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1.5
shot 08-directo-toque; a11y 08-directo-toque
check "directo: tocar enseña los controles" 'visible "Cerrar" 08-directo-toque'

xcrun simctl spawn "$UDID" log show --last 3m --predicate "process CONTAINS 'VIBE'" > "$OUT/log.txt" 2>/dev/null || true
if grep -q "panicked at" "$OUT/log.txt"; then echo "MAL  el núcleo entró en pánico"; grep -A3 "panicked at" "$OUT/log.txt" | head; fail=1; fi
exit $fail
