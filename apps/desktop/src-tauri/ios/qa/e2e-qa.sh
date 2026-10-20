#!/usr/bin/env bash
# Prueba de punta a punta de la app en un iPhone/iPad simulado, con toques reales (idb) y
# el proveedor falso (tools/proveedor-falso) corriendo en el propio Mac. La app arranca en
# modo de pruebas (`VIBE_QA_E2E`, ver `commands::seed_qa_e2e`): sin cuenta, con un perfil y
# el proveedor falso ya sincronizándose.
#
# Uso: e2e-qa.sh <udid> <ruta.app> <dir-salida>
set -uo pipefail
UDID="$1"
APP="$2"
OUT="$3"
mkdir -p "$OUT"
fail=0
step=0

shot() { step=$((step + 1)); xcrun simctl io "$UDID" screenshot "$OUT/$(printf %02d $step)-$1.png" >/dev/null 2>&1; }
# Lo que se ve: lo nativo según idb (reproductor) y, delante, lo de dentro del WebView,
# que idb no ve; la página lo escribe en `qa-ui.json` (ver `src/lib/qaSnapshot.ts`).
WEB_MAP=""
a11y() {
  idb ui describe-all --udid "$UDID" --json > "$OUT/a11y-nativo.json" 2>/dev/null || echo "[]" > "$OUT/a11y-nativo.json"
  if [ -z "$WEB_MAP" ]; then
    local data
    data=$(xcrun simctl get_app_container "$UDID" "$bundle" data 2>/dev/null)
    [ -n "$data" ] && WEB_MAP=$(find "$data" -name qa-ui.json 2>/dev/null | head -1)
  fi
  python3 - "$OUT/a11y-nativo.json" "${WEB_MAP:-/nonexistent}" "$OUT/a11y.json" <<'PY'
import json, sys
def load(p):
    try:
        return json.load(open(p))
    except Exception:
        return []
native, web = load(sys.argv[1]), load(sys.argv[2])
json.dump(native + web, open(sys.argv[3], "w"), ensure_ascii=False)
PY
}
# Centro del primer elemento cuya etiqueta es exactamente $1 (o la contiene, con $2=contiene).
find_label() {
  python3 - "$OUT/a11y.json" "$1" "${2:-exacta}" <<'PY'
import json, sys
try:
    items = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
want, mode = sys.argv[2], sys.argv[3]
for it in items:
    label = (it.get("AXLabel") or "").strip()
    if (label == want) if mode == "exacta" else (want.lower() in label.lower()):
        f = it.get("frame") or {}
        print(f"{f.get('x', 0) + f.get('width', 0) / 2:.0f} {f.get('y', 0) + f.get('height', 0) / 2:.0f} {f.get('y', 0) + f.get('height', 0):.0f}")
        break
PY
}
tap_label() {
  local xy
  for _ in $(seq 1 "${3:-15}"); do
    a11y
    xy=$(find_label "$1" "${2:-exacta}")
    if [ -n "$xy" ]; then
      set -- $xy
      idb ui tap --udid "$UDID" "$1" "$2"
      return 0
    fi
    sleep 1
  done
  echo "  (no encontré «$1»)"
  return 1
}
visible() { a11y; [ -n "$(find_label "$1" "${2:-exacta}")" ]; }
wait_visible() { for _ in $(seq 1 "${3:-20}"); do if visible "$1" "${2:-exacta}"; then return 0; fi; sleep 1; done; return 1; }
check() { if eval "$2"; then echo "ok   $1"; else echo "MAL  $1"; fail=1; fi; }
active_cons() { curl -s "http://127.0.0.1:8899/player_api.php?username=qa&password=qa" | python3 -c "import json,sys; print(json.load(sys.stdin)['user_info']['active_cons'])" 2>/dev/null || echo "?"; }

bundle=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$APP/Info.plist")
xcrun simctl install "$UDID" "$APP"
SIMCTL_CHILD_VIBE_QA_E2E=1 SIMCTL_CHILD_VIBE_QA_PROVIDER_URL="http://127.0.0.1:8899" SIMCTL_CHILD_RUST_BACKTRACE=1 \
  xcrun simctl launch "$UDID" "$bundle" >/dev/null
sleep 15
# Tamaño de la pantalla según idb. Si idb no responde, se dice y se para: sin esto la
# prueba seguía a ciegas con un tamaño inventado (pasó con el iPad).
W=""; H=""
for _ in $(seq 1 20); do
  a11y
  read -r W H < <(python3 -c "import json;d=json.load(open('$OUT/a11y-nativo.json'));f=d[0]['frame'];print(int(f['width']),int(f['height']))" 2>/dev/null || true)
  [ -n "$W" ] && break
  sleep 3
done
if [ -z "$W" ]; then
  echo "MAL  idb no responde: no se puede probar este aparato"
  shot sin-idb
  exit 1
fi
echo "pantalla: ${W}x${H}"

# Perfil.
shot perfiles
tap_label "QA" contiene 30; sleep 4
shot inicio
check "entra en Inicio tras elegir perfil" 'wait_visible "Inicio" exacta 20'

# La barra de pestañas tiene que estar pegada abajo (el usuario la veía muy arriba).
a11y
read -r _ _ tab_bottom < <(find_label "Navegación principal")
echo "barra de pestañas: borde inferior en ${tab_bottom:-?} de ${H}"
check "la barra de pestañas llega abajo del todo" '[ -n "$tab_bottom" ] && [ $((H - tab_bottom)) -le 40 ]'

# Pestañas.
for tab in Directo Películas Series Buscar; do
  tap_label "$tab"; sleep 3; shot "pestana-$tab"
done
# El simulador pregunta por el dictado la primera vez que se enfoca un campo de texto.
tap_label "Not Now" exacta 3 >/dev/null; sleep 1
if [ "$W" -lt 768 ]; then
  tap_label "Más"; sleep 1; shot "menu-mas"
  tap_label "Guía"; sleep 3; shot "guia"
  tap_label "Más"; sleep 1; tap_label "Ajustes"; sleep 3; shot "ajustes"
else
  # En el iPad, Guía y Ajustes están en la barra lateral.
  tap_label "Guía"; sleep 3; shot "guia"
  tap_label "Ajustes"; sleep 3; shot "ajustes"
fi

# Película: ficha, reproducir, +30 s, cerrar.
tap_label "Películas"; sleep 3
# Mantener pulsada una portada abre el menú de acciones (Mi lista…).
a11y; xy=$(find_label "Toy Story" contiene)
if [ -n "$xy" ]; then set -- $xy; idb ui tap --udid "$UDID" --duration 0.9 "$1" "$2"; fi
sleep 1; shot menu-mantener
check "mantener pulsada una portada abre su menú" 'wait_visible "Mi lista" contiene 5'
tap_label "Cancelar" exacta 5; sleep 1
tap_label "Toy Story" contiene 10 || tap_label "Del revés" contiene 5
sleep 3; shot ficha-pelicula
tap_label "Reproducir" contiene 10
check "película: se abre el reproductor" 'wait_visible "Cerrar" exacta 25'
shot reproductor-pelicula
sleep 6; idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
tap_label "Adelante 30 segundos" exacta 5; sleep 5
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
shot tras-avanzar
check "película: avanzar 30 s sigue reproduciendo" 'visible "Cerrar"'
check "película: una sola conexión con el proveedor" '[ "$(active_cons)" -le 1 ] 2>/dev/null'
tap_label "Cerrar" exacta 5; sleep 3
shot tras-cerrar-pelicula
check "película: cerrar vuelve a la app" '! visible "Cerrar"'

# Directo: abrir canal, pausar, ir al directo, zapping.
tap_label "Directo"; sleep 3
tap_label "Reloj" contiene 10 || tap_label "1 " contiene 5
check "directo: se abre el reproductor" 'wait_visible "Cerrar" exacta 25'
shot reproductor-directo
sleep 5; idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
tap_label "Reproducir o pausar" exacta 5; sleep 12
tap_label "Reproducir o pausar" exacta 5; sleep 3
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
shot directo-tras-pausa
check "directo: tras la pausa va por detrás del directo" 'visible "Ir al directo" contiene'
tap_label "Ir al directo" contiene 5; sleep 4
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
shot directo-de-vuelta
check "directo: «Ir al directo» vuelve al directo" 'visible "EN DIRECTO" contiene'
# Zapping: deslizar hacia arriba en el centro.
idb ui swipe --udid "$UDID" $((W / 2)) $((H * 2 / 3)) $((W / 2)) $((H / 3)) 2>/dev/null; sleep 6
idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
shot directo-tras-zapping
check "directo: el zapping sigue en el reproductor" 'visible "Cerrar"'
check "directo: una sola conexión con el proveedor" '[ "$(active_cons)" -le 1 ] 2>/dev/null'
tap_label "Cerrar" exacta 5; sleep 3

# Serie: la ficha ofrece «▶ Ver T1 · E1» y ese botón abre el episodio.
tap_label "Series"; sleep 3
tap_label "Breaking Bad" contiene 10; sleep 3; shot ficha-serie
check "serie: la ficha ofrece ver el primer episodio" 'wait_visible "T1 · E1" contiene 10'
tap_label "T1 · E1" contiene 5
check "serie: se abre el reproductor del episodio" 'wait_visible "Cerrar" exacta 25'
shot reproductor-episodio
sleep 6; idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1
tap_label "Cerrar" exacta 5; sleep 3
shot tras-cerrar-episodio
check "serie: cerrar vuelve a la ficha" '! visible "Cerrar" && visible "Breaking Bad" contiene'

# Buscar escribiendo (en el iPhone el campo está abajo y sube con el teclado).
tap_label "Buscar"; sleep 3
tap_label "Campo de búsqueda" exacta 5; sleep 1
tap_label "Not Now" exacta 2 >/dev/null; sleep 1
idb ui text --udid "$UDID" "toy" 2>/dev/null; sleep 3
shot buscar-escribiendo
check "buscar: al escribir salen resultados" 'wait_visible "Toy Story" contiene 8'

# Ajustes, desplazado hasta abajo.
if [ "$W" -lt 768 ]; then tap_label "Más"; sleep 1; fi
tap_label "Ajustes"; sleep 3
check "ajustes: enseña la compilación" 'wait_visible "Compilación" contiene 5'
idb ui swipe --udid "$UDID" $((W / 2)) $((H * 3 / 4)) $((W / 2)) $((H / 4)) 2>/dev/null; sleep 1
shot ajustes-abajo
shot fin

xcrun simctl spawn "$UDID" log show --last 6m --predicate "process CONTAINS 'VIBE'" > "$OUT/log.txt" 2>/dev/null || true
if grep -q "panicked at" "$OUT/log.txt"; then echo "MAL  el núcleo entró en pánico"; grep -A3 "panicked at" "$OUT/log.txt" | head; fail=1; fi
exit $fail
