import { useEffect, useState } from "react";

/** Misma condición que la variante `desk:` de `globals.css`: hay sitio para la barra lateral. */
const DESK_QUERY = "(min-width: 768px) and (min-height: 560px)";

/** `true` si la pantalla es de "escritorio" (PC, iPad): barra lateral en vez de pestañas. */
export function useIsDesk(): boolean {
  const [desk, setDesk] = useState(() => window.matchMedia(DESK_QUERY).matches);
  useEffect(() => {
    const media = window.matchMedia(DESK_QUERY);
    const onChange = () => setDesk(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);
  return desk;
}

/** Píxeles que tapa ahora el teclado en pantalla (iOS, F15). En el WebView, lo fijo abajo
 * se queda detrás del teclado: quien quiera ir pegado encima de él suma este valor. */
export function useKeyboardInset(): number {
  const [inset, setInset] = useState(0);
  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) return;
    const update = () => setInset(Math.max(0, Math.round(window.innerHeight - viewport.height - viewport.offsetTop)));
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    update();
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
    };
  }, []);
  return inset;
}
