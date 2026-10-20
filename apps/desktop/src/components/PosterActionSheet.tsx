import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useFavorites } from "../lib/favorites";
import { HeartIcon } from "./icons";

type PosterActionSheetProps = {
  title: string;
  onOpen: () => void;
  onClose: () => void;
  /** Película o serie: con esto la hoja ofrece añadir o quitar de "Mi lista". */
  menuItem?: { kind: "movie" | "series"; id: number };
  /** Conmutador ya resuelto por quien pinta la tarjeta (rejillas): tiene prioridad. */
  favorite?: { isFavorite: boolean; onToggle: () => void };
  onRemove?: () => void;
};

/** Hoja de acciones al mantener pulsada una portada (iPhone/iPad, F15), como el menú
 * contextual de Netflix o la app TV: abrir, Mi lista y quitar de "Continuar viendo".
 * Se pinta en `body` para que el `scale` de la tarjeta no la deforme. */
export function PosterActionSheet({ title, onOpen, onClose, menuItem, favorite, onRemove }: PosterActionSheetProps) {
  const { isFavorite, toggleFavorite } = useFavorites();
  const fav =
    favorite ??
    (menuItem
      ? { isFavorite: isFavorite(menuItem.kind, menuItem.id), onToggle: () => toggleFavorite(menuItem.kind, menuItem.id, !isFavorite(menuItem.kind, menuItem.id)) }
      : undefined);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Backspace") {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  function run(action: () => void) {
    onClose();
    action();
  }

  const itemClass = "flex w-full items-center gap-3 rounded-xl px-4 py-3.5 text-left text-base text-neutral-100 outline-none active:bg-neutral-800 focus-visible:bg-neutral-800";

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-end justify-center sm:items-center" role="dialog" aria-modal="true" aria-label={title}>
      <button type="button" aria-label="Cerrar" tabIndex={-1} onClick={onClose} className="sheet-fade absolute inset-0 bg-black/60" />
      <div className="sheet-up relative w-full max-w-md rounded-t-2xl bg-neutral-900 p-2 pb-[calc(0.5rem+env(safe-area-inset-bottom))] shadow-2xl sm:rounded-2xl sm:pb-2">
        <div className="mx-auto mb-1 mt-1 h-1 w-10 rounded-full bg-neutral-700 sm:hidden" />
        <p className="truncate px-4 pb-2 pt-2 text-sm font-semibold text-neutral-400">{title}</p>
        <button type="button" autoFocus onClick={() => run(onOpen)} className={itemClass}>
          <span className="w-6 text-center">▶</span>
          Abrir
        </button>
        {fav && (
          <button type="button" onClick={() => run(fav.onToggle)} className={itemClass}>
            <HeartIcon filled={fav.isFavorite} className={`h-6 w-6 ${fav.isFavorite ? "text-red-500" : ""}`} />
            {fav.isFavorite ? "Quitar de Mi lista" : "Añadir a Mi lista"}
          </button>
        )}
        {onRemove && (
          <button type="button" onClick={() => run(onRemove)} className={itemClass}>
            <span className="w-6 text-center">✕</span>
            Quitar de Continuar viendo
          </button>
        )}
        <button type="button" onClick={onClose} className={`${itemClass} mt-1 justify-center bg-neutral-800/60 font-medium`}>
          Cancelar
        </button>
      </div>
    </div>,
    document.body,
  );
}
