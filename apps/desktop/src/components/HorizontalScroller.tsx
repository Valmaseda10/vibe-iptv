import { useCallback, useEffect, useRef, useState, type HTMLAttributes, type ReactNode, type RefObject } from "react";
import { ChevronIcon } from "./icons";

type HorizontalScrollerProps = Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
  children: ReactNode;
  /** Para quien necesite el elemento que se desplaza (p. ej. `Row`, que mueve el foco). */
  trackRef?: RefObject<HTMLDivElement | null>;
  /** Clases del contenedor exterior (el que lleva las flechas). */
  outerClassName?: string;
};

/** Pista horizontal sin barra de desplazamiento visible. Con ratón aparecen flechas ‹ ›
 * en los bordes al pasar por encima (solo si hay más contenido hacia ese lado); el
 * teclado sigue moviendo el foco y el trackpad, Mayús+rueda y el dedo desplazan solos. */
export function HorizontalScroller({ children, trackRef, outerClassName, className, ...rest }: HorizontalScrollerProps) {
  const ownRef = useRef<HTMLDivElement | null>(null);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);

  const setRef = useCallback(
    (node: HTMLDivElement | null) => {
      ownRef.current = node;
      if (trackRef) trackRef.current = node;
    },
    [trackRef],
  );

  const measure = useCallback(() => {
    const track = ownRef.current;
    if (!track) return;
    setCanLeft(track.scrollLeft > 2);
    setCanRight(track.scrollLeft + track.clientWidth < track.scrollWidth - 2);
  }, []);

  useEffect(() => {
    const track = ownRef.current;
    if (!track) return;
    measure();
    track.addEventListener("scroll", measure, { passive: true });
    const observer = new ResizeObserver(measure);
    observer.observe(track);
    // Los hijos pueden llegar después (paginación, imágenes): también cambia el ancho total.
    const mutations = new MutationObserver(measure);
    mutations.observe(track, { childList: true });
    return () => {
      track.removeEventListener("scroll", measure);
      observer.disconnect();
      mutations.disconnect();
    };
  }, [measure]);

  function scrollByPage(direction: 1 | -1) {
    const track = ownRef.current;
    if (!track) return;
    track.scrollBy({ left: direction * track.clientWidth * 0.85, behavior: "smooth" });
  }

  const arrow =
    "absolute inset-y-0 z-10 hidden w-10 items-center justify-center text-white opacity-0 transition-opacity duration-150 group-hover/scroller:opacity-100 hover:bg-black/60 desk:flex ios:!hidden";

  return (
    <div className={`group/scroller relative ${outerClassName ?? ""}`}>
      <div ref={setRef} className={`[scrollbar-width:none] [&::-webkit-scrollbar]:hidden ${className ?? ""}`} {...rest}>
        {children}
      </div>
      {canLeft && (
        <button type="button" tabIndex={-1} aria-label="Desplazar a la izquierda" onClick={() => scrollByPage(-1)} className={`${arrow} left-0 bg-gradient-to-r from-black/70 to-transparent`}>
          <ChevronIcon className="h-7 w-7 rotate-180" />
        </button>
      )}
      {canRight && (
        <button type="button" tabIndex={-1} aria-label="Desplazar a la derecha" onClick={() => scrollByPage(1)} className={`${arrow} right-0 bg-gradient-to-l from-black/70 to-transparent`}>
          <ChevronIcon className="h-7 w-7" />
        </button>
      )}
    </div>
  );
}
