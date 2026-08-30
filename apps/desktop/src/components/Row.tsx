import { useRef, type ReactNode } from "react";

type RowProps = {
  title: string;
  children: ReactNode;
};

/** Fila horizontal desplazable. Flecha izquierda/derecha mueve el foco entre tarjetas
 * (además del scroll nativo por rueda/touch); cada hijo debe ser un elemento enfocable
 * (un `<button>`, como `PosterCard`). */
export function Row({ title, children }: RowProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  function onKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const track = trackRef.current;
    if (!track) return;
    const focusable = Array.from(track.querySelectorAll<HTMLElement>("button"));
    const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
    if (currentIndex === -1) return;
    e.preventDefault();
    const nextIndex = e.key === "ArrowRight" ? Math.min(currentIndex + 1, focusable.length - 1) : Math.max(currentIndex - 1, 0);
    focusable[nextIndex]?.focus();
    focusable[nextIndex]?.scrollIntoView({ behavior: "smooth", inline: "center", block: "nearest" });
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="px-6 text-lg font-semibold text-neutral-200 sm:px-10">{title}</h2>
      <div
        ref={trackRef}
        onKeyDown={onKeyDown}
        className="flex gap-4 overflow-x-auto px-6 pb-2 sm:px-10 [scrollbar-width:thin]"
      >
        {children}
      </div>
    </section>
  );
}
