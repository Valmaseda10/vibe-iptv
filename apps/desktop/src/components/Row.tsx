import { useRef, type ReactNode } from "react";

type RowProps = {
  title: string;
  children: ReactNode;
};

/** Marca cada pista de fila para que las flechas arriba/abajo puedan saltar entre filas
 * distintas de la misma pantalla (ver `focusVerticalNeighbor`) — sin esto, moverse entre
 * "Continuar viendo" y "Recomendado para ti" solo era posible con Tab, que en un mando a
 * distancia no existe. */
const ROW_TRACK_SELECTOR = "[data-row-track]";

/** Lo que recorren las flechas dentro de una fila: la tarjeta, no sus botones
 * secundarios (♥, ✕), que se alcanzan con Tab. Sin esto, la flecha derecha caía en el ✕
 * de "Quitar de Continuar viendo" en vez de en la tarjeta siguiente. Filas cuyos hijos no
 * son `PosterCard` recurren a cualquier botón. */
function rowItems(track: HTMLElement): HTMLElement[] {
  const cards = Array.from(track.querySelectorAll<HTMLElement>("[data-card]"));
  return cards.length > 0 ? cards : Array.from(track.querySelectorAll<HTMLElement>("button"));
}

/** Salta desde la tarjeta `currentIndex` de `currentTrack` a la fila anterior/siguiente
 * (`direction`: 1 abajo, -1 arriba) del documento entero — deliberadamente global en vez
 * de recibir las filas vecinas como prop: así funciona igual entre `Row`s que no se
 * conocen entre sí (p. ej. Inicio y una fila "Más como esto" en otra pantalla), sin que
 * cada pantalla tenga que cablear el orden a mano. Mantiene la misma columna cuando
 * puede (como en cualquier rejilla de TV real); si la fila destino tiene menos tarjetas,
 * se ajusta a la última en vez de no hacer nada. */
function focusVerticalNeighbor(currentTrack: HTMLElement, currentIndex: number, direction: 1 | -1) {
  const tracks = Array.from(document.querySelectorAll<HTMLElement>(ROW_TRACK_SELECTOR));
  const trackIndex = tracks.indexOf(currentTrack);
  if (trackIndex === -1) return;
  const targetTrack = tracks[trackIndex + direction];
  if (!targetTrack) return; // primera o última fila: sin salto, no hay a dónde ir.
  const focusable = rowItems(targetTrack);
  if (focusable.length === 0) return;
  const target = focusable[Math.min(currentIndex, focusable.length - 1)];
  if (!target) return;
  target.focus();
  target.scrollIntoView({ behavior: "smooth", inline: "center", block: "nearest" });
}

/** Fila horizontal desplazable. Izquierda/derecha mueve el foco entre tarjetas de la
 * misma fila; arriba/abajo salta a la fila anterior/siguiente de la pantalla (además del
 * scroll nativo por rueda/touch en ambos ejes). Cada hijo debe ser un elemento enfocable
 * (un `<button>`, como `PosterCard`). */
export function Row({ title, children }: RowProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  function onKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    const track = trackRef.current;
    if (!track) return;
    const focusable = rowItems(track);
    const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
    if (currentIndex === -1) return;

    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      const nextIndex = e.key === "ArrowRight" ? Math.min(currentIndex + 1, focusable.length - 1) : Math.max(currentIndex - 1, 0);
      focusable[nextIndex]?.focus();
      focusable[nextIndex]?.scrollIntoView({ behavior: "smooth", inline: "center", block: "nearest" });
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      focusVerticalNeighbor(track, currentIndex, e.key === "ArrowDown" ? 1 : -1);
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="px-4 text-lg font-semibold text-neutral-200 sm:px-10">{title}</h2>
      <div
        ref={trackRef}
        data-row-track
        onKeyDown={onKeyDown}
        className="flex gap-3 overflow-x-auto px-4 pb-2 sm:gap-4 sm:px-10 [scrollbar-width:thin]"
      >
        {children}
      </div>
    </section>
  );
}
