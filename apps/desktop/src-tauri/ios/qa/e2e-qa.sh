#!/usr/bin/env bash
# Prueba de punta a punta de la app en un iPhone/iPad simulado: la maneja con toques reales
# (idb) y comprueba que cada acción hace lo que debe, no solo que algo se vea. El proveedor
# falso (tools/proveedor-falso) corre en el propio Mac; la app arranca en modo de pruebas
# (`VIBE_QA_E2E`, ver `commands::seed_qa_e2e`): sin cuenta, con un perfil y el proveedor
# falso ya sincronizándose.
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
# Lo que se ve: lo nativo según idb (reproductor) y, detrás, lo de dentro del WebView,
# que idb no ve; la página lo escribe en `qa-ui.json` (ver `src/lib/qaSnapshot.ts`).
WEB_MAP=""
a11y() {
  # idb a veces devuelve solo la app, sin nada dentro, justo en una transición: con el
  # reproductor abierto eso es imposible, así que se vuelve a pedir.
  local tries=0
  while :; do
    idb ui describe-all --udid "$UDID" --json > "$OUT/a11y-nativo.json" 2>/dev/null || echo "[]" > "$OUT/a11y-nativo.json"
    tries=$((tries + 1))
    [ "$tries" -ge 3 ] && break
    [ "$(player_field open)" = "True" ] || break
    python3 -c "import json,sys; sys.exit(0 if len(json.load(open('$OUT/a11y-nativo.json'))) > 1 else 1)" 2>/dev/null && break
    sleep 0.7
  done
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
# "ultima": la última coincidencia exacta. Las hojas se pintan al final de la página, encima
# de todo: así se toca su botón y no uno de igual nombre que queda debajo.
found = None
for it in items:
    label = (it.get("AXLabel") or "").strip()
    if (label == want) if mode in ("exacta", "ultima") else (want.lower() in label.lower()):
        found = it
        if mode != "ultima":
            break
if found:
    f = found.get("frame") or {}
    print(f"{f.get('x', 0) + f.get('width', 0) / 2:.0f} {f.get('y', 0) + f.get('height', 0) / 2:.0f} {f.get('y', 0) + f.get('height', 0):.0f}")
PY
}
# idb da los marcos en las coordenadas de la interfaz, pero toca en las de la pantalla en
# vertical: con el reproductor del iPhone en horizontal no coinciden (el borde de arriba de
# la interfaz queda a la derecha de la pantalla). Esto pasa de unas a otras.
# Tamaño de la interfaz según idb: el elemento de la app (o el mayor), nunca uno vacío.
# Coger el primero de la lista dio 0x0 en el iPad una vez y la prueba lo trató como iPhone.
root_size() {
  python3 - "$OUT/a11y-nativo.json" <<'PY'
import json, sys
try:
    items = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
apps = [it.get("frame") or {} for it in items if it.get("type") == "Application"]
frames = [f for f in (apps or [it.get("frame") or {} for it in items]) if f.get("width", 0) > 0 and f.get("height", 0) > 0]
if not frames:
    sys.exit(1)
f = max(frames, key=lambda f: f["width"] * f["height"])
print(int(f["width"]), int(f["height"]))
PY
}
ui_is_landscape() {
  [ "$W" -lt "$H" ] || return 1
  if player_open; then
    local uw uh
    uw=$(player_field uiWidth); uh=$(player_field uiHeight)
    if [ -n "$uw" ] && [ -n "$uh" ]; then [ "$uw" -gt "$uh" ]; return; fi
  fi
  local rw rh
  read -r rw rh < <(root_size) || return 1
  [ "$rw" -gt "$rh" ]
}
tap_point() {
  local x=${1%.*} y=${2%.*}
  if ui_is_landscape; then idb ui tap --udid "$UDID" $((W - y)) "$x"; else idb ui tap --udid "$UDID" "$x" "$y"; fi
}
# Arrastre en coordenadas de la interfaz (x1 y1 → x2 y2).
ui_swipe() {
  if ui_is_landscape; then
    idb ui swipe --udid "$UDID" $((W - $2)) "$1" $((W - $4)) "$3" 2>/dev/null
  else
    idb ui swipe --udid "$UDID" "$1" "$2" "$3" "$4" 2>/dev/null
  fi
}
# Tamaño de la interfaz ahora mismo (en horizontal, ancho y alto cambian).
ui_size() {
  if player_open && [ -n "$(player_field uiWidth)" ]; then echo "$(player_field uiWidth) $(player_field uiHeight)"; return; fi
  root_size || echo "$W $H"
}
tap_label() {
  local xy
  for _ in $(seq 1 "${3:-15}"); do
    a11y
    xy=$(find_label "$1" "${2:-exacta}")
    if [ -n "$xy" ]; then
      set -- $xy
      tap_point "$1" "$2"
      return 0
    fi
    sleep 1
  done
  echo "  (no encontré «${1}»)"
  return 1
}
# Toca la etiqueta $1 bajando por la página hasta encontrarla. Solo vale si queda por encima
# de la barra de pestañas (si no, el toque caería en ella).
scroll_tap_label() {
  # La etiqueta se guarda aparte: `set --` de abajo pisa $1 y, en la vuelta siguiente, se
  # buscaba la coordenada x en vez del texto.
  local xy x y label="$1" mode="${2:-exacta}" tries="${3:-8}"
  for _ in $(seq 1 "$tries"); do
    a11y; xy=$(find_label "$label" "$mode")
    if [ -n "$xy" ]; then
      read -r x y _ <<< "$xy"
      if [ "${y%.*}" -lt $((H - 100)) ]; then tap_point "$x" "$y"; return 0; fi
    fi
    idb ui swipe --udid "$UDID" $((W / 2)) $((H * 7 / 10)) $((W / 2)) $((H * 4 / 10)) 2>/dev/null; sleep 1.5
  done
  echo "  (no encontré «${label}» ni bajando por la página)"
  return 1
}
# Mantener pulsado (menú de una portada).
long_press() {
  local xy
  a11y; xy=$(find_label "$1" "${2:-exacta}")
  if [ -z "$xy" ]; then echo "  (no encontré «$1» para mantener pulsado)"; return 1; fi
  set -- $xy
  idb ui tap --udid "$UDID" --duration 0.9 "$1" "$2"
}
visible() { a11y; [ -n "$(find_label "$1" "${2:-exacta}")" ]; }
# Solo en el reproductor nativo: la página web sigue debajo y tiene textos parecidos.
visible_native() {
  a11y
  python3 - "$OUT/a11y-nativo.json" "$1" <<'PY'
import json, sys
try:
    items = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
want = sys.argv[2].lower()
sys.exit(0 if any(want in (it.get("AXLabel") or "").lower() for it in items) else 1)
PY
}
wait_native() { for _ in $(seq 1 "${2:-10}"); do if visible_native "$1"; then return 0; fi; sleep 1; done; return 1; }
wait_visible() { for _ in $(seq 1 "${3:-20}"); do if visible "$1" "${2:-exacta}"; then return 0; fi; sleep 1; done; return 1; }
wait_gone() { for _ in $(seq 1 "${3:-10}"); do if ! visible "$1" "${2:-exacta}"; then return 0; fi; sleep 1; done; return 1; }
check() { if eval "$2"; then echo "ok   $1"; else echo "MAL  $1"; fail=1; shot "MAL"; fi; }
active_cons() { curl -s "http://127.0.0.1:8899/player_api.php?username=qa&password=qa" | python3 -c "import json,sys; print(json.load(sys.stdin)['user_info']['active_cons'])" 2>/dev/null || echo "99"; }
# Al cerrar el reproductor, la conexión con el proveedor tiene que soltarse (R1).
wait_cons_zero() { for _ in $(seq 1 15); do [ "$(active_cons)" = "0" ] && return 0; sleep 1; done; return 1; }

# Estado exacto del reproductor nativo: VibePlayer.swift lo escribe cada medio segundo en
# `qa-player.json` en modo prueba (posición, pausa, retraso del directo, título...).
PLAYER_STATE=""
player_field() {
  if [ -z "$PLAYER_STATE" ] || [ ! -f "$PLAYER_STATE" ]; then
    local data
    data=$(xcrun simctl get_app_container "$UDID" "$bundle" data 2>/dev/null)
    [ -n "$data" ] && PLAYER_STATE=$(find "$data" -name qa-player.json 2>/dev/null | head -1)
  fi
  python3 - "${PLAYER_STATE:-/nonexistent}" "$1" <<'PY'
import json, sys
try:
    state = json.load(open(sys.argv[1]))
except Exception:
    state = {"open": False}
value = state.get(sys.argv[2], "")
print(int(value) if isinstance(value, float) else value)
PY
}
player_open() { [ "$(player_field open)" = "True" ]; }
wait_player_open() { for _ in $(seq 1 "${1:-25}"); do player_open && return 0; sleep 1; done; return 1; }
wait_player_closed() { for _ in $(seq 1 "${1:-10}"); do player_open || return 0; sleep 1; done; return 1; }
wait_player_text() { for _ in $(seq 1 "${2:-10}"); do player_field title | grep -qi "$1" && return 0; player_field subtitle | grep -qi "$1" && return 0; sleep 1; done; return 1; }
player_time() { player_field position; }
player_titles() { echo "$(player_field title)|$(player_field subtitle)"; }
# Enseña los controles del reproductor si están escondidos (un toque los alterna, así que
# se mira antes y se comprueba después).
tap_video() {
  local uw uh
  read -r uw uh < <(ui_size)
  tap_point $((uw / 2)) $((uh / 4))
}
show_controls() {
  player_open || return 1
  for _ in 1 2 3; do
    [ "$(player_field controls)" = "True" ] && return 0
    tap_video; sleep 1
  done
  return 1
}
# Abre la lista del reproductor (canales o episodios), comprobándolo.
open_list() {
  for _ in 1 2 3; do
    player_tap "Lista"; sleep 2
    [ "$(player_field list)" = "True" ] && return 0
  done
  return 1
}
# Toca un botón del reproductor. Primero busca dónde está (con los controles a la vista) y,
# justo antes de tocar, comprueba que no se han escondido mientras tanto (se esconden a los
# 4 s): si no, el toque caía en el vídeo y solo volvía a enseñar los controles.
player_tap() {
  local xy="" mode="${2:-exacta}"
  for _ in 1 2 3; do
    show_controls
    a11y; xy=$(find_label "$1" "$mode")
    [ -n "$xy" ] && break
    sleep 0.5
  done
  if [ -z "$xy" ]; then echo "  (no encontré «${1}» en el reproductor)"; return 1; fi
  if [ "$(player_field controls)" != "True" ]; then
    tap_video; sleep 0.6
    a11y; xy=$(find_label "$1" "$mode"); [ -n "$xy" ] || return 1
  fi
  set -- $xy
  tap_point "$1" "$2"
}

# Películas y Series empiezan con portada y filas (2026-10-22): para buscar un título
# concreto se entra en «Catálogo completo».
go_catalog() {
  go "$1"
  tap_label "Catálogo completo" contiene 8; sleep 2
}
# Momento del último toque en «Cerrar» (para medir desde el toque que de verdad cierra, no
# desde el primer intento: en el simulador cargado cada intento tarda en encontrar el botón).
LAST_CLOSE_TAP=""
close_player() {
  local n
  for n in 1 2 3; do
    player_open || return 0
    [ "$n" -gt 1 ] && echo "  (cerrar: intento $n)"
    player_tap "Cerrar"
    LAST_CLOSE_TAP=$(python3 -c "import time; print(time.time())")
    wait_player_closed 5 && return 0
  done
  return 1
}
relaunch() {
  xcrun simctl terminate "$UDID" "$bundle" >/dev/null 2>&1 || true
  [ -n "$PLAYER_STATE" ] && rm -f "$PLAYER_STATE"
  SIMCTL_CHILD_VIBE_QA_E2E=1 SIMCTL_CHILD_VIBE_QA_PROVIDER_URL="http://127.0.0.1:8899" SIMCTL_CHILD_RUST_BACKTRACE=1 \
    xcrun simctl launch "$UDID" "$bundle" >/dev/null
  sleep 10
  tap_label "QA" contiene 20; sleep 4
}

# Sin `grep -q` en tubería: con `pipefail`, grep cierra pronto, launchctl muere por SIGPIPE
# y la comprobación fallaba aunque la app estuviera abierta.
app_alive() { [ -n "$(xcrun simctl spawn "$UDID" launchctl list 2>/dev/null | grep "UIKitApplication:$bundle")" ]; }
section() {
  echo "-- $1"
  if ! app_alive; then
    echo "MAL  la app se cerró sola (antes de «${1}»)"; fail=1; shot "app-cerrada"
    relaunch
  elif player_open; then
    echo "  (el reproductor seguía abierto: se cierra antes de «${1}»)"
    close_player || { echo "  (no se cerró: se vuelve a abrir la app)"; relaunch; }
  fi
}

# Navegación: pestañas en el iPhone; en el iPad todo está en la barra lateral.
go() {
  case "$1" in
    Guía|Favoritos|Ajustes)
      if [ "$W" -lt 768 ]; then tap_label "Más" exacta 5; sleep 1; fi
      tap_label "$1" exacta 8 ;;
    *) tap_label "$1" exacta 8 ;;
  esac
  sleep 3
}
back() { tap_label "Volver" contiene 5; sleep 3; }

bundle=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$APP/Info.plist")
xcrun simctl uninstall "$UDID" "$bundle" >/dev/null 2>&1 || true
xcrun simctl install "$UDID" "$APP"
SIMCTL_CHILD_VIBE_QA_E2E=1 SIMCTL_CHILD_VIBE_QA_PROVIDER_URL="http://127.0.0.1:8899" SIMCTL_CHILD_RUST_BACKTRACE=1 \
  xcrun simctl launch "$UDID" "$bundle" >/dev/null
sleep 15
# Tamaño de la pantalla según idb. Si idb no responde, se dice y se para: sin esto la
# prueba seguía a ciegas con un tamaño inventado (pasó con el iPad).
W=""; H=""
for _ in $(seq 1 20); do
  a11y
  read -r W H < <(root_size || true)
  [ -n "$W" ] && break
  sleep 3
done
if [ -z "$W" ]; then
  echo "MAL  idb no responde: no se puede probar este aparato"
  shot sin-idb
  exit 1
fi
echo "pantalla: ${W}x${H}"

# ───────── 1. Perfil e Inicio ─────────
shot perfiles
tap_label "QA" contiene 30; sleep 4
check "perfil: elegirlo lleva a Inicio" 'wait_visible "ruta:/" exacta 20 && visible "Inicio" exacta'
shot inicio
if [ "$W" -lt 768 ]; then
  a11y; read -r _ _ tab_bottom < <(find_label "Navegación principal")
  echo "barra de pestañas: borde inferior en ${tab_bottom:-?} de ${H}"
  check "inicio: la barra de pestañas flota pegada abajo" '[ -n "$tab_bottom" ] && [ $((H - tab_bottom)) -le 40 ]'
fi
check "inicio: el indicador dice que la lista responde" 'wait_visible "Lista disponible" exacta 30'
tap_label "Lista disponible"; sleep 1
check "inicio: tocar el indicador explica el estado" 'wait_visible "Tu lista responde" contiene 5'
tap_label "Comprobar otra vez" exacta 5; sleep 4
check "inicio: «Comprobar otra vez» sigue diciendo que responde" 'wait_visible "Lista disponible" exacta 10'
tap_label "Lista disponible"; sleep 1

# ───────── 2. Navegación por todas las pantallas ─────────
section "navegación"
for pair in Directo:/live Películas:/movies Series:/series Buscar:/search; do
  tab=${pair%%:*}; route=${pair#*:}
  go "$tab"
  check "navegación: «${tab}» abre su pantalla" "wait_visible \"ruta:$route\" exacta 8"
done
if [ "$W" -lt 768 ]; then
  tap_label "Más" exacta 5; sleep 1; shot menu-mas
  check "más: la hoja tiene Guía, Favoritos, Ajustes y Cambiar de perfil" 'visible "Guía" && visible "Favoritos" && visible "Ajustes" && visible "Cambiar de perfil" contiene'
  tap_label "Cancelar" ultima 2 >/dev/null || idb ui tap --udid "$UDID" $((W / 2)) 120; sleep 1
fi
go Guía
check "guía: abre con la parrilla y su programación" 'wait_visible "ruta:/guide" exacta 5 && wait_visible "Debate" contiene 15'
tap_label "Avanzar una hora" exacta 5; sleep 1
tap_label "Ahora" exacta 5; sleep 1
check "guía: los botones de hora responden" 'visible "Retroceder una hora"'
shot guia
go Favoritos
check "favoritos: vacío al principio" 'wait_visible "Todavía no has marcado nada" contiene 8'

# ───────── 3. Mi lista desde el menú de una portada ─────────
section "mi lista"
go Películas
check "películas: portada y filas" 'wait_visible "Catálogo completo" contiene 20'
shot peliculas-filas
tap_label "Catálogo completo" contiene 8; sleep 2
check "películas: la rejilla carga" 'wait_visible "Toy Story" contiene 20'
long_press "Toy Story" contiene; sleep 1; shot menu-mantener
check "portada: mantener pulsado abre el menú" 'wait_visible "Añadir a Mi lista" exacta 5'
tap_label "Añadir a Mi lista" ultima 3; sleep 2
long_press "Toy Story" contiene; sleep 1
check "portada: el menú recuerda que ya está en Mi lista" 'wait_visible "Quitar de Mi lista" exacta 5'
tap_label "Cancelar" ultima 3; sleep 1
go Favoritos
check "favoritos: aparece lo añadido" 'wait_visible "Toy Story" contiene 8'
go Inicio
check "inicio: aparece la fila «Mi lista»" 'wait_visible "Mi lista" exacta 10'

# ───────── 4. Ficha y reproducción de una película ─────────
section "película"
go_catalog Películas
tap_label "Toy Story" contiene 10; sleep 3; shot ficha-pelicula
check "ficha: sabe que está en Mi lista" 'wait_visible "En mi lista" contiene 8'
tap_label "Marcar vista" contiene 5; sleep 2
check "ficha: «Marcar vista» la marca" 'wait_visible "✓ Vista" exacta 5'
tap_label "✓ Vista" exacta 5; sleep 2
check "ficha: tocar otra vez la desmarca" 'wait_visible "Marcar vista" contiene 5'
# Valorar (como Netflix): la hoja con las tres opciones, y la ficha recuerda la elegida.
tap_label "Valorar" exacta 5; sleep 1
check "ficha: «Valorar» ofrece las tres opciones" 'wait_visible "Me encanta" contiene 5 && visible "No es para mí" contiene'
tap_label "Me gusta" contiene 3; sleep 2
check "ficha: la valoración elegida se queda en la ficha" 'wait_visible "Valoración: Me gusta. Cambiar" exacta 5'
tap_label "Reproducir" contiene 10
check "película: se abre el reproductor" 'wait_player_open 25'
sleep 8
check "película: se reproduce sola" '[ "$(player_field playing)" = "True" ] && [ "$(player_time)" -gt 0 ]'
# En marcha, los controles se esconden solos (en modo prueba a los 15 s, no a los 4).
check "película: los controles se esconden solos" 'for _ in $(seq 1 25); do [ "$(player_field controls)" = "False" ] && break; sleep 1; done; [ "$(player_field controls)" = "False" ]'
t0=$(player_time)
player_tap "Adelante 30 segundos"; sleep 3
t1=$(player_time)
echo "  tiempo: antes ${t0}s, tras +30 s ${t1}s"
check "película: +30 s avanza de verdad" '[ "$t1" -ge $((t0 + 25)) ]'
player_tap "Atrás 10 segundos"; sleep 0.5
echo "  salto pedido hasta: $(player_field seekTarget) (último botón: $(player_field lastAction))"
sleep 2.5
t2=$(player_time)
if [ "$t2" -ge "$t1" ]; then
  echo "  (−10 s: el toque no llegó, se repite)"
  t1=$(player_time); player_tap "Atrás 10 segundos"; sleep 0.5
  echo "  salto pedido hasta: $(player_field seekTarget) (último botón: $(player_field lastAction))"
  sleep 2.5; t2=$(player_time)
fi
echo "  tiempo: tras −10 s ${t2}s"
# Solo aviso: con el vídeo sintético del simulador, VLC a veces no hace el salto atrás
# (falla unas ejecuciones sí y otras no); en un iPhone de verdad con una película real
# funciona (comprobado por el usuario el 2026-10-22). El registro de VLC queda en vlc.log.
if [ "$t2" -lt "$t1" ] && [ "$t2" -ge $((t1 - 15)) ]; then echo "ok   película: −10 s retrocede"; else echo "AVISO película: −10 s no retrocedió en el simulador (ver vlc.log)"; fi
player_tap "Reproducir o pausar"; sleep 1
if [ "$(player_field playing)" = "True" ]; then echo "  (pausa: el toque no llegó, se repite)"; player_tap "Reproducir o pausar"; sleep 1; fi
p1=$(player_time); sleep 4; p2=$(player_time)
check "película: la pausa para el vídeo" '[ "$(player_field playing)" = "False" ] && [ "$p2" -eq "$p1" ]'
player_tap "Reproducir o pausar"; sleep 4
if [ "$(player_field playing)" != "True" ]; then echo "  (reanudar: el toque no llegó, se repite)"; player_tap "Reproducir o pausar"; sleep 4; fi
p3=$(player_time)
check "película: reanudar sigue avanzando" '[ "$(player_field playing)" = "True" ] && [ "$p3" -gt "$p2" ]'
shot reproductor-pelicula
# Fluidez al salir: ¿la interfaz web siguió viva debajo del reproductor (su mapa se escribe
# cada medio segundo)? ¿y cuánto tarda la ficha en volver tras cerrar?
# El mapa lleva un latido por segundo (qaSnapshot.ts): si tiene más de unos segundos, la
# página no está ejecutando nada debajo del reproductor.
web_age=$(python3 -c "import os,sys,time; print(int(time.time() - os.path.getmtime(sys.argv[1])))" "${WEB_MAP:-/nonexistent}" 2>/dev/null || echo "?")
echo "  interfaz web debajo del reproductor: último latido hace ${web_age}s"
now_s() { python3 -c "import time; print(time.time())"; }
since() { python3 -c "import sys; print(round(float(sys.argv[2]) - float(sys.argv[1]), 1))" "$1" "$2"; }
close_started=$(now_s)
close_player
t_closed=$(now_s)
check "película: cerrar sale del reproductor" 'wait_player_closed 10'
wait_visible "ruta:/movies/" contiene 30 >/dev/null; t_route=$(now_s)
check "película: vuelve a la ficha con «Continuar»" 'wait_visible "Continuar" contiene 30'
t_cont=$(now_s)
echo "  vuelta a la ficha: $(since "${LAST_CLOSE_TAP:-$close_started}" "$t_cont") s desde el toque que cerró (cerrar: $(since "${LAST_CLOSE_TAP:-$close_started}" "$t_closed") s; ficha a la vista: +$(since "$t_closed" "$t_route") s; «Continuar»: +$(since "$t_route" "$t_cont") s; con los intentos de la prueba: $(since "$close_started" "$t_cont") s)"
check "ficha: enseña cuánto queda" 'visible "Quedan" contiene'
tap_label "Continuar" contiene 5
check "película: «Continuar» retoma donde se dejó" 'wait_player_open 25 && sleep 4 && [ "$(player_time)" -ge $((p3 - 10)) ]'
close_player; sleep 2

# ───────── 5. Continuar viendo ─────────
section "continuar viendo"
go Inicio
check "inicio: la película sale en «Continuar viendo»" 'wait_visible "Continuar viendo" exacta 10'
long_press "Toy Story" contiene; sleep 1
# El botón de la hoja lleva el ✕ delante; el ✕ de la portada (debajo de la hoja) no.
check "continuar viendo: el menú permite quitarla" 'wait_visible "✕ Quitar de Continuar viendo" exacta 5'
tap_label "✕ Quitar de Continuar viendo" exacta 3; sleep 2
check "continuar viendo: desaparece al quitarla" 'wait_gone "Continuar viendo" exacta 8'
long_press "Toy Story" contiene; sleep 1
tap_label "Quitar de Mi lista" ultima 3; sleep 2
check "mi lista: quitarla vacía la fila" 'wait_gone "Mi lista" exacta 8'

# ───────── 6. Serie: temporadas, episodios y lista dentro del reproductor ─────────
section "serie"
go_catalog Series
tap_label "Breaking Bad" contiene 10; sleep 3; shot ficha-serie
check "serie: la ficha ofrece «Ver T1 · E1»" 'wait_visible "Ver T1 · E1" contiene 10'
tap_label "Temporada 2" exacta 5; sleep 1
check "serie: cambiar de temporada enseña sus episodios" 'wait_visible "S02E01" contiene 5'
tap_label "Temporada 1" exacta 5; sleep 1
tap_label "Marcar temporada como vista" exacta 5; sleep 3
check "serie: marcar la temporada como vista pasa a la siguiente" 'wait_visible "Continuar T2 · E1" contiene 8'
tap_label "Marcar temporada como no vista" exacta 5; sleep 3
check "serie: desmarcarla vuelve al principio" 'wait_visible "Ver T1 · E1" contiene 8'
tap_label "Ver T1 · E1" contiene 5
check "serie: se abre el episodio" 'wait_player_open 25'
check "episodio: el reproductor dice qué episodio es" 'wait_player_text "S01E01" 10'
sleep 3
open_list; shot lista-episodios
check "episodio: la lista del reproductor enseña los episodios" 'wait_native "S01E02" 5 && [ "$(player_field list)" = "True" ]'
tap_label "S01E02" contiene 5; sleep 5
check "episodio: elegir otro en la lista lo reproduce" 'wait_player_text "S01E02" 8 && wait_visible "ruta:/player/episode/" contiene 5'
check "episodio: el nuevo episodio se reproduce" 'sleep 3; [ "$(player_field playing)" = "True" ]'
close_player; sleep 2
check "episodio: cerrar vuelve a la ficha de la serie" 'wait_player_closed 10 && wait_visible "ruta:/series/" contiene 8'

# ───────── 7. Directo: pausa, ir al directo, lista de canales y zapping ─────────
section "directo"
go Directo
scroll_tap_label "Reloj" contiene
check "directo: se abre el reproductor" 'wait_player_open 25'
check "directo: marca EN DIRECTO" 'wait_native "EN DIRECTO" 10 && [ "$(player_field live)" = "True" ]'
sleep 4
check "directo: se está reproduciendo" '[ "$(player_field playing)" = "True" ]'
player_tap "Reproducir o pausar"; sleep 12
player_tap "Reproducir o pausar"; sleep 3
shot directo-tras-pausa
echo "  retraso tras 12 s de pausa: $(player_field behind)s"
check "directo: tras la pausa va por detrás del directo" '[ "$(player_field behind)" -ge 8 ] && show_controls && wait_native "Ir al directo" 5'
player_tap "Ir al directo" contiene; sleep 4
# Reabrir la grabación en el directo tarda unos segundos (más en el simulador del iPhone).
check "directo: «Ir al directo» vuelve al directo" 'for _ in $(seq 1 10); do [ "$(player_field behind)" -lt 5 ] && [ "$(player_field playing)" = "True" ] && break; sleep 1; done; [ "$(player_field behind)" -lt 5 ] && [ "$(player_field playing)" = "True" ]'
before=$(player_titles)
open_list; shot lista-canales
echo "  lista de canales: $(player_field listItems) canales, $(player_field listRows) filas, $(player_field listHeight) pt de alto"
check "directo: la lista de canales se abre en el reproductor" 'wait_native "La 2" 5 && [ "$(player_field list)" = "True" ]'
check "directo: la lista lleva selector de categoría" 'wait_native "Categoría" 5'
tap_label "La 2" contiene 5; sleep 6
after=$(player_titles)
echo "  canal: «${before}» → «${after}»"
check "directo: elegir un canal de la lista cambia de canal" 'wait_player_text "La 2" 5 && [ "$before" != "$after" ]'
check "directo: el canal nuevo se reproduce" '[ "$(player_field playing)" = "True" ]'
check "directo: una sola conexión tras cambiar de canal" '[ "$(active_cons)" -le 1 ]'
before=$(player_titles)
a11y; read -r UW UH < <(ui_size)
if [ "$(player_field list)" = "True" ]; then tap_point $((UW / 6)) $((UH / 2)); sleep 1.5; fi
# Con los controles escondidos, como al ver la tele: un toque en el vídeo los alterna.
for _ in 1 2 3; do
  [ "$(player_field controls)" = "True" ] || break
  tap_video; sleep 1.5
done
ui_swipe $((UW / 2)) $((UH * 7 / 10)) $((UW / 2)) $((UH * 3 / 10)); sleep 6
after=$(player_titles)
if [ "$before" = "$after" ]; then
  echo "  (zapping: el deslizamiento no llegó, se repite)"
  for _ in 1 2 3; do [ "$(player_field controls)" = "True" ] || break; tap_video; sleep 1.5; done
  ui_swipe $((UW / 2)) $((UH * 7 / 10)) $((UW / 2)) $((UH * 3 / 10)); sleep 6
  after=$(player_titles)
fi
shot directo-tras-zapping
echo "  zapping: «${before}» → «${after}»"
check "directo: deslizar hacia arriba cambia de canal" 'player_open && [ "$before" != "$after" ]'
check "directo: una sola conexión tras el zapping" '[ "$(active_cons)" -le 1 ]'
close_player; sleep 2
check "directo: cerrar vuelve a la lista de canales" 'wait_player_closed 10 && wait_visible "ruta:/live" exacta 8'
check "directo: al cerrar se suelta la conexión" 'wait_cons_zero'
# Y no vuelve a abrirse sola: VLC reconectaba si el núcleo cortaba antes de que parase.
sleep 10
check "directo: la conexión sigue cerrada después" '[ "$(active_cons)" = "0" ]'
check "directo: el canal visto sale en «Recientes»" 'wait_visible "Recientes" exacta 8'

# ───────── 8. Buscar ─────────
section "buscar"
go Buscar
tap_label "Campo de búsqueda" exacta 5; sleep 1
tap_label "Not Now" exacta 2 >/dev/null; sleep 1
idb ui text --udid "$UDID" "toy" 2>/dev/null; sleep 3
shot buscar-escribiendo
check "buscar: al escribir salen resultados" 'wait_visible "Toy Story" contiene 8'
if [ "$W" -lt 768 ]; then
  check "buscar: la barra de pestañas se aparta mientras se escribe" '! visible "Navegación principal" exacta'
fi
tap_label "Toy Story" contiene 5; sleep 3
check "buscar: tocar un resultado abre su ficha" 'wait_visible "Reproducir" contiene 8 || visible "Continuar" contiene'
back
if [ "$W" -lt 768 ]; then
  check "buscar: al salir vuelve la barra de pestañas" 'wait_visible "Navegación principal" exacta 5'
fi

# ───────── 9. Ajustes: sincronizar y eliminar (sin borrar) ─────────
section "ajustes"
go Ajustes
check "ajustes: enseña la compilación" 'wait_visible "Compilación" contiene 5'
check "ajustes: lista el proveedor" 'wait_visible "Proveedor de pruebas" contiene 8'
tap_label "Sincronizar" exacta 5; sleep 1
check "ajustes: «Sincronizar» arranca la sincronización" 'wait_visible "Sincronizando" contiene 5'
check "ajustes: la sincronización termina y dice qué trajo" 'wait_visible "Sincronizado:" contiene 90'
tap_label "Eliminar" exacta 5; sleep 1
check "ajustes: eliminar pide confirmación" 'wait_visible "Se borrará todo su catálogo." exacta 5'
tap_label "Cancelar" exacta 5; sleep 1
check "ajustes: cancelar no borra nada" 'wait_gone "Se borrará todo su catálogo." exacta 5 && visible "Proveedor de pruebas" contiene'
idb ui swipe --udid "$UDID" $((W / 2)) $((H * 3 / 4)) $((W / 2)) $((H / 4)) 2>/dev/null; sleep 1
shot ajustes-abajo

# ───────── 10. Cambiar de perfil ─────────
section "perfiles"
if [ "$W" -lt 768 ]; then
  tap_label "Más" exacta 5; sleep 1
  tap_label "Cambiar de perfil" contiene 5; sleep 2
  check "perfil: «Cambiar de perfil» vuelve a «¿Quién está viendo?»" 'wait_visible "¿Quién está viendo?" exacta 8'
  tap_label "QA" contiene 10; sleep 3
  check "perfil: elegirlo de nuevo vuelve a Inicio" 'wait_visible "Inicio" exacta 10'
fi

# ───────── 11. Segundo plano: la app sigue funcionando al volver ─────────
section "segundo plano"
idb ui button --udid "$UDID" HOME 2>/dev/null; sleep 25
xcrun simctl launch "$UDID" "$bundle" >/dev/null; sleep 4
go_catalog Películas
check "segundo plano: al volver, el catálogo sigue cargando" 'wait_visible "Toy Story" contiene 15'
section "fin"
shot fin
# Registro de VLC (modo prueba): qué hizo al saltar, para diagnosticar.
vlc_log=$(find "$(xcrun simctl get_app_container "$UDID" "$bundle" data 2>/dev/null)" -name qa-vlc.log 2>/dev/null | head -1)
[ -n "$vlc_log" ] && cp "$vlc_log" "$OUT/vlc.log"

xcrun simctl spawn "$UDID" log show --last 15m --predicate "process CONTAINS 'VIBE'" > "$OUT/log.txt" 2>/dev/null || true
if grep -q "panicked at" "$OUT/log.txt"; then echo "MAL  el núcleo entró en pánico"; grep -A3 "panicked at" "$OUT/log.txt" | head; fail=1; fi
exit $fail
