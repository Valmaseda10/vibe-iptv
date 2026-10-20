import { useFavorites } from "../lib/favorites";
import { GlassSheet, SHEET_ROW } from "./GlassSheet";
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
 * contextual de Netflix o la app TV: abrir, Mi lista y quitar de "Continuar viendo". */
export function PosterActionSheet({ title, onOpen, onClose, menuItem, favorite, onRemove }: PosterActionSheetProps) {
  const { isFavorite, toggleFavorite } = useFavorites();
  const fav =
    favorite ??
    (menuItem
      ? { isFavorite: isFavorite(menuItem.kind, menuItem.id), onToggle: () => toggleFavorite(menuItem.kind, menuItem.id, !isFavorite(menuItem.kind, menuItem.id)) }
      : undefined);

  function run(action: () => void) {
    onClose();
    action();
  }

  return (
    <GlassSheet title={title} onClose={onClose}>
      <button type="button" autoFocus onClick={() => run(onOpen)} className={SHEET_ROW}>
        <span className="w-6 text-center">▶</span>
        Abrir
      </button>
      {fav && (
        <button type="button" onClick={() => run(fav.onToggle)} className={SHEET_ROW}>
          <HeartIcon filled={fav.isFavorite} className={`h-6 w-6 ${fav.isFavorite ? "text-red-500" : ""}`} />
          {fav.isFavorite ? "Quitar de Mi lista" : "Añadir a Mi lista"}
        </button>
      )}
      {onRemove && (
        <button type="button" onClick={() => run(onRemove)} className={SHEET_ROW}>
          <span className="w-6 text-center">✕</span>
          Quitar de Continuar viendo
        </button>
      )}
      <button type="button" onClick={onClose} className={`${SHEET_ROW} mt-1 justify-center bg-white/10 font-semibold`}>
        Cancelar
      </button>
    </GlassSheet>
  );
}
