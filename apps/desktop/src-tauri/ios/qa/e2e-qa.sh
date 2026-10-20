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

# Reproductor nativo: enseña los controles si están escondidos (un toque los alterna).
show_controls() { visible "Cerrar" || { idb ui tap --udid "$UDID" $((W / 2)) $((H / 2)); sleep 1; }; }
# Segundos que marca el reproductor (etiqueta de tiempo de la barra de abajo).
player_time() {
  show_controls
  python3 - "$OUT/a11y-nativo.json" <<'PY'
import json, re, sys
try:
    items = json.load(open(sys.argv[1]))
except Exception:
    items = []
for it in items:
    label = (it.get("AXLabel") or "").strip()
    m = re.fullmatch(r"(?:(\d+):)?(\d+):(\d\d)", label)
    if m:
        h, mm, ss = m.groups()
        print(int(h or 0) * 3600 + int(mm) * 60 + int(ss))
        break
else:
    print(-1)
PY
}
# Título que enseña el reproductor (para ver que el zapping cambió de canal).
player_titles() {
  a11y
  python3 - "$OUT/a11y-nativo.json" <<'PY'
import json, re, sys
items = json.load(open(sys.argv[1]))
labels = [i.get("AXLabel") or "" for i in items]
print("|".join(sorted(l for l in labels if l and not re.fullmatch(r"-?(\d+:)?\d+:\d\d", l) and "Ir al directo" not in l)))
PY
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
for pair in Directo:/live Películas:/movies Series:/series Buscar:/search; do
  tab=${pair%%:*}; route=${pair#*:}
  go "$tab"
  check "navegación: «$tab» abre su pantalla" "wait_visible \"ruta:$route\" exacta 8"
done
if [ "$W" -lt 768 ]; then
  tap_label "Más" exacta 5; sleep 1; shot menu-mas
  check "más: la hoja tiene Guía, Favoritos, Ajustes y Cambiar de perfil" 'visible "Guía" && visible "Favoritos" && visible "Ajustes" && visible "Cambiar de perfil"'
  tap_label "Cancelar" exacta 2 >/dev/null || idb ui tap --udid "$UDID" $((W / 2)) 120; sleep 1
fi
go Guía
check "guía: abre con la parrilla de canales" 'wait_visible "ruta:/guide" exacta 5 && wait_visible "Reloj" contiene 15'
tap_label "Avanzar una hora" exacta 5; sleep 1
tap_label "Ahora" exacta 5; sleep 1
check "guía: los botones de hora responden" 'visible "Retroceder una hora"'
shot guia
go Favoritos
check "favoritos: vacío al principio" 'wait_visible "Todavía no has marcado nada" contiene 8'

# ───────── 3. Mi lista desde el menú de una portada ─────────
go Películas
check "películas: la rejilla carga" 'wait_visible "Toy Story" contiene 20'
long_press "Toy Story" contiene; sleep 1; shot menu-mantener
check "portada: mantener pulsado abre el menú" 'wait_visible "Añadir a Mi lista" exacta 5'
tap_label "Añadir a Mi lista" exacta 3; sleep 2
long_press "Toy Story" contiene; sleep 1
check "portada: el menú recuerda que ya está en Mi lista" 'wait_visible "Quitar de Mi lista" exacta 5'
tap_label "Cancelar" exacta 3; sleep 1
go Favoritos
check "favoritos: aparece lo añadido" 'wait_visible "Toy Story" contiene 8'
go Inicio
check "inicio: aparece la fila «Mi lista»" 'wait_visible "Mi lista" exacta 10'

# ───────── 4. Ficha y reproducción de una película ─────────
go Películas
tap_label "Toy Story" contiene 10; sleep 3; shot ficha-pelicula
check "ficha: sabe que está en Mi lista" 'wait_visible "En mi lista" contiene 8'
tap_label "Marcar vista" contiene 5; sleep 2
check "ficha: «Marcar vista» la marca" 'wait_visible "✓ Vista" exacta 5'
tap_label "✓ Vista" exacta 5; sleep 2
check "ficha: tocar otra vez la desmarca" 'wait_visible "Marcar vista" contiene 5'
tap_label "Reproducir" contiene 10
check "película: se abre el reproductor" 'wait_visible "Cerrar" exacta 25'
sleep 8
t0=$(player_time)
show_controls; tap_label "Adelante 30 segundos" exacta 5; sleep 4
t1=$(player_time)
echo "  tiempo: antes ${t0}s, tras +30 s ${t1}s"
check "película: +30 s avanza de verdad" '[ "$t0" -ge 0 ] && [ "$t1" -ge $((t0 + 20)) ]'
show_controls; tap_label "Atrás 10 segundos" exacta 5; sleep 3
t2=$(player_time)
check "película: −10 s retrocede" '[ "$t2" -ge 0 ] && [ "$t2" -lt "$t1" ]'
show_controls; tap_label "Reproducir o pausar" exacta 5; sleep 1
p1=$(player_time); sleep 5; p2=$(player_time)
check "película: la pausa para el vídeo" '[ "$p1" -ge 0 ] && [ $((p2 - p1)) -le 1 ]'
show_controls; tap_label "Reproducir o pausar" exacta 5; sleep 5
p3=$(player_time)
check "película: reanudar sigue avanzando" '[ "$p3" -gt "$p2" ]'
check "película: una sola conexión con el proveedor" '[ "$(active_cons)" -le 1 ]'
shot reproductor-pelicula
show_controls; tap_label "Cerrar" exacta 5; sleep 3
check "película: cerrar vuelve a la ficha" '! visible "Cerrar" && wait_visible "Continuar" contiene 8'
check "ficha: enseña cuánto queda" 'visible "Quedan" contiene'
check "película: al cerrar se suelta la conexión" 'wait_cons_zero'
tap_label "Continuar" contiene 5
check "película: «Continuar» retoma donde se dejó" 'wait_visible "Cerrar" exacta 25 && [ "$(player_time)" -ge $((p3 - 10)) ]'
show_controls; tap_label "Cerrar" exacta 5; sleep 3

# ───────── 5. Continuar viendo ─────────
go Inicio
check "inicio: la película sale en «Continuar viendo»" 'wait_visible "Continuar viendo" exacta 10'
long_press "Toy Story" contiene; sleep 1
check "continuar viendo: el menú permite quitarla" 'wait_visible "Quitar de Continuar viendo" exacta 5'
tap_label "Quitar de Continuar viendo" exacta 3; sleep 2
check "continuar viendo: desaparece al quitarla" 'wait_gone "Continuar viendo" exacta 8'
long_press "Toy Story" contiene; sleep 1
tap_label "Quitar de Mi lista" exacta 3; sleep 2
check "mi lista: quitarla vacía la fila" 'wait_gone "Mi lista" exacta 8'

# ───────── 6. Serie: temporadas, episodios y lista dentro del reproductor ─────────
go Series
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
check "serie: se abre el episodio" 'wait_visible "Cerrar" exacta 25'
check "episodio: el reproductor dice qué episodio es" 'wait_native "S01E01" 10'
sleep 3; show_controls
tap_label "Lista" exacta 5; sleep 2; shot lista-episodios
check "episodio: la lista del reproductor enseña los episodios" 'wait_native "S01E02" 5'
tap_label "S01E02" contiene 5; sleep 5
check "episodio: elegir otro en la lista lo reproduce" 'show_controls; wait_native "S01E02" 8 && ! visible_native "S01E01"'
check "episodio: una sola conexión con el proveedor" '[ "$(active_cons)" -le 1 ]'
show_controls; tap_label "Cerrar" exacta 5; sleep 3
check "episodio: cerrar vuelve a la ficha de la serie" '! visible "Cerrar" && wait_visible "Breaking Bad" contiene 8'
check "episodio: al cerrar se suelta la conexión" 'wait_cons_zero'

# ───────── 7. Directo: pausa, ir al directo, lista de canales y zapping ─────────
go Directo
tap_label "Reloj" contiene 10
check "directo: se abre el reproductor" 'wait_visible "Cerrar" exacta 25'
check "directo: marca EN DIRECTO" 'wait_native "EN DIRECTO" 10'
sleep 4; show_controls
tap_label "Reproducir o pausar" exacta 5; sleep 12
show_controls; tap_label "Reproducir o pausar" exacta 5; sleep 3
show_controls; shot directo-tras-pausa
check "directo: tras la pausa va por detrás del directo" 'wait_native "Ir al directo" 5'
tap_label "Ir al directo" contiene 5; sleep 4
show_controls
check "directo: «Ir al directo» vuelve al directo" 'wait_native "EN DIRECTO" 5 && ! visible_native "Ir al directo"'
before=$(player_titles)
show_controls; tap_label "Lista" exacta 5; sleep 2; shot lista-canales
check "directo: la lista de canales se abre en el reproductor" 'wait_native "La 2" 5'
tap_label "La 2" contiene 5; sleep 6
after=$(player_titles)
check "directo: elegir un canal de la lista cambia de canal" '[ "$before" != "$after" ]'
check "directo: una sola conexión tras cambiar de canal" '[ "$(active_cons)" -le 1 ]'
before=$(player_titles)
idb ui swipe --udid "$UDID" $((W / 2)) $((H * 2 / 3)) $((W / 2)) $((H / 3)) 2>/dev/null; sleep 6
show_controls; after=$(player_titles); shot directo-tras-zapping
check "directo: deslizar hacia arriba cambia de canal" 'visible_native "Cerrar" && [ "$before" != "$after" ]'
check "directo: una sola conexión tras el zapping" '[ "$(active_cons)" -le 1 ]'
show_controls; tap_label "Cerrar" exacta 5; sleep 3
check "directo: cerrar vuelve a la lista de canales" '! visible_native "Cerrar" && wait_visible "ruta:/live" exacta 8'
check "directo: al cerrar se suelta la conexión" 'wait_cons_zero'
check "directo: el canal visto sale en «Recientes»" 'wait_visible "Recientes" exacta 8'

# ───────── 8. Buscar ─────────
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
if [ "$W" -lt 768 ]; then
  tap_label "Más" exacta 5; sleep 1
  tap_label "Cambiar de perfil" exacta 5; sleep 2
  check "perfil: «Cambiar de perfil» vuelve a «¿Quién está viendo?»" 'wait_visible "¿Quién está viendo?" exacta 8'
  tap_label "QA" contiene 10; sleep 3
  check "perfil: elegirlo de nuevo vuelve a Inicio" 'wait_visible "Inicio" exacta 10'
fi

# ───────── 11. Segundo plano: la app sigue funcionando al volver ─────────
idb ui button --udid "$UDID" HOME 2>/dev/null; sleep 25
xcrun simctl launch "$UDID" "$bundle" >/dev/null; sleep 4
go Películas
check "segundo plano: al volver, el catálogo sigue cargando" 'wait_visible "Toy Story" contiene 15'
shot fin

xcrun simctl spawn "$UDID" log show --last 15m --predicate "process CONTAINS 'VIBE'" > "$OUT/log.txt" 2>/dev/null || true
if grep -q "panicked at" "$OUT/log.txt"; then echo "MAL  el núcleo entró en pánico"; grep -A3 "panicked at" "$OUT/log.txt" | head; fail=1; fi
exit $fail
