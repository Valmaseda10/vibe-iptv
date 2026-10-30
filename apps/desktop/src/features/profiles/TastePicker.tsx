import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { imagePath, useApiBase } from "../../lib/api-client";
import type { BrowseItem } from "../../lib/api-types";

/** Mínimo de títulos para seguir. */
const MIN_PICKS = 3;

type TastePickerProps = {
  items: BrowseItem[];
  /** Los elegidos (cada uno cuenta como «Me gusta»). */
  onDone: (picked: BrowseItem[]) => void;
  onSkip: () => void;
  saving: boolean;
};

/** «Elige lo que te gusta», como al empezar en Netflix: con un perfil sin gustos todavía,
 * unos cuantos títulos variados para tocar los que gustan; con ellos las recomendaciones
 * son personales desde el primer día. Se puede saltar. Enter/espacio eligen, Esc salta. */
export function TastePicker({ items, onDone, onSkip, saving }: TastePickerProps) {
  const { data: base } = useApiBase();
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const firstRef = useRef<HTMLButtonElement>(null);
  const key = (item: BrowseItem) => `${item.kind}:${item.id}`;

  useEffect(() => {
    firstRef.current?.focus();
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onSkip();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onSkip]);

  function toggle(item: BrowseItem) {
    setPicked((current) => {
      const next = new Set(current);
      if (next.has(key(item))) next.delete(key(item));
      else next.add(key(item));
      return next;
    });
  }

  const count = picked.size;
  // En `body`: dentro de la pantalla, su animación de entrada encierra lo `fixed` y la barra
  // de pestañas del iPhone taparía el botón «Listo».
  return createPortal(
    <div className="fixed inset-0 z-[60] flex flex-col bg-neutral-950 ios:bg-black" role="dialog" aria-modal="true" aria-labelledby="taste-title">
      <header className="px-4 pb-4 pt-[max(1.5rem,env(safe-area-inset-top))] sm:px-10 sm:pt-10">
        <h1 id="taste-title" className="text-2xl font-bold text-white sm:text-4xl">¿Qué te gusta?</h1>
        <p className="mt-2 max-w-2xl text-sm text-neutral-300 sm:text-base">
          Elige al menos {MIN_PICKS} títulos que te gusten y te recomendaremos más como ellos. Puedes cambiarlo cuando quieras valorando lo que veas.
        </p>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-6 sm:px-10">
        <ul className="grid grid-cols-3 gap-3 sm:grid-cols-4 sm:gap-4 lg:grid-cols-6" aria-label="Títulos para elegir">
          {items.map((item, index) => {
            const selected = picked.has(key(item));
            const src = imagePath(base, item.poster_path, "w342");
            return (
              <li key={key(item)}>
                <button
                  ref={index === 0 ? firstRef : undefined}
                  type="button"
                  onClick={() => toggle(item)}
                  aria-pressed={selected}
                  aria-label={`${item.clean_title}${item.kind === "series" ? " (serie)" : ""}`}
                  className={`group relative block aspect-[2/3] w-full overflow-hidden rounded-lg bg-neutral-800 outline-none transition focus-visible:ring-4 focus-visible:ring-white ${
                    selected ? "ring-4 ring-red-600" : "hover:brightness-110"
                  }`}
                >
                  {src ? (
                    <img src={src} alt="" loading="lazy" className={`h-full w-full object-cover transition ${selected ? "scale-95 opacity-60" : ""}`} />
                  ) : (
                    <span className="flex h-full items-center justify-center p-2 text-center text-xs text-neutral-300">{item.clean_title}</span>
                  )}
                  {selected && (
                    <span className="absolute inset-0 flex items-center justify-center">
                      <span className="flex h-12 w-12 items-center justify-center rounded-full bg-red-600 text-2xl font-bold text-white shadow-lg">✓</span>
                    </span>
                  )}
                  <span className="absolute left-1.5 top-1.5 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-medium text-neutral-200">
                    {item.kind === "series" ? "Serie" : "Película"}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </div>

      <footer className="flex items-center justify-between gap-3 border-t border-white/10 bg-neutral-950/95 px-4 pb-[max(1rem,env(safe-area-inset-bottom))] pt-4 backdrop-blur sm:px-10 ios:bg-black/90">
        <button type="button" onClick={onSkip} className="rounded-lg px-4 py-2.5 text-sm text-neutral-300 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white/60">
          Ahora no
        </button>
        <div className="flex items-center gap-4">
          <span className="hidden text-sm text-neutral-400 sm:inline" aria-live="polite">
            {count < MIN_PICKS ? `Elige ${MIN_PICKS - count} más` : `${count} elegidos`}
          </span>
          <button
            type="button"
            disabled={count < MIN_PICKS || saving}
            onClick={() => onDone(items.filter((item) => picked.has(key(item))))}
            className="rounded-lg bg-red-600 px-6 py-2.5 font-semibold text-white outline-none transition hover:bg-red-500 focus-visible:ring-4 focus-visible:ring-red-400 disabled:cursor-not-allowed disabled:bg-neutral-700 disabled:text-neutral-400 ios:rounded-full"
          >
            {saving ? "Guardando…" : count < MIN_PICKS ? `Elige ${MIN_PICKS - count} más` : "Listo"}
          </button>
        </div>
      </footer>
    </div>,
    document.body,
  );
}
