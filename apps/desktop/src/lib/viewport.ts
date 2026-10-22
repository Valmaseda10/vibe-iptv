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

/** Dónde acaba ahora la zona visible de la pantalla (encima del teclado, si está abierto),
 * en las mismas coordenadas que `position: fixed; top`. iOS (F15): calcularlo como «lo que
 * tapa el teclado desde abajo» fallaba cuando iOS reservaba además las zonas de la hora y
 * del indicador de inicio, y el campo de Buscar quedaba flotando lejos del teclado. */
export function useVisibleBottom(): number {
  const [bottom, setBottom] = useState(() => window.visualViewport ? window.visualViewport.offsetTop + window.visualViewport.height : window.innerHeight);
  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) return;
    const update = () => setBottom(Math.round(viewport.offsetTop + viewport.height));
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    update();
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
    };
  }, []);
  return bottom;
}
