import { invoke } from "@tauri-apps/api/core";

/** Solo en las pruebas del simulador (`ios/qa/e2e-qa.sh`, compilación de depuración con
 * `VIBE_QA_E2E`): idb no ve los botones dentro del WebView, así que la página escribe
 * cada medio segundo un mapa de lo que se ve, con el mismo formato que `idb ui
 * describe-all`. En la app instalada `qa_e2e_enabled` es `false` y no hace nada. */
export function startQaSnapshots() {
  invoke<boolean>("qa_e2e_enabled")
    .then((enabled) => {
      if (!enabled) return;
      let last = "";
      window.setInterval(() => {
        const json = JSON.stringify(snapshot());
        if (json === last) return;
        last = json;
        invoke("qa_ui_snapshot", { json }).catch(() => {});
      }, 500);
    })
    .catch(() => {});
}

function snapshot() {
  const items: { AXLabel: string; frame: { x: number; y: number; width: number; height: number } }[] = [];
  const nodes = document.querySelectorAll<HTMLElement>("button, a, [role=button], [role=menuitem], [role=tab], [aria-label], h1, h2, h3, p, span, input");
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1 || rect.bottom <= 0 || rect.top >= window.innerHeight) continue;
    const label = (node.getAttribute("aria-label") ?? node.innerText ?? "").replace(/\s+/g, " ").trim();
    if (!label || label.length > 120) continue;
    items.push({ AXLabel: label, frame: { x: rect.left, y: rect.top, width: rect.width, height: rect.height } });
  }
  // La ruta actual, para que la prueba sepa en qué pantalla está de verdad.
  items.push({ AXLabel: `ruta:${window.location.pathname}`, frame: { x: 0, y: 0, width: 0, height: 0 } });
  // Latido: cambia cada segundo, así el mapa se reescribe aunque la pantalla no cambie y la
  // prueba sabe si la página sigue viva (p. ej. debajo del reproductor).
  items.push({ AXLabel: `latido:${Math.floor(Date.now() / 1000)}`, frame: { x: 0, y: 0, width: 0, height: 0 } });
  return items;
}
