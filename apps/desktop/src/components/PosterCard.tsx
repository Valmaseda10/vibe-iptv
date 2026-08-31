import { useApiBase, imagePath } from "../lib/api-client";
import { HeartIcon } from "./icons";

type PosterCardProps = {
  title: string;
  posterPath: string | null;
  subtitle?: string;
  onOpen: () => void;
  /** Índice dentro de una rejilla virtualizada (`VirtualPosterGrid`): permite localizar
   * y enfocar la tarjeta por teclado tras un `scrollToIndex`, ya que puede no existir
   * todavía en el DOM en el momento de pulsar la flecha. */
  gridIndex?: number;
  /** Conmutador de favorito en la esquina de la tarjeta (F8). Omitido en sitios donde
   * no aplica (p. ej. "continuar viendo", donde el ítem ya lo eligió el usuario). Es un
   * <button> hermano del de abrir, nunca anidado dentro — dos <button> uno dentro de
   * otro es HTML inválido y rompe la navegación por Tab. */
  favorite?: { isFavorite: boolean; onToggle: () => void };
  /** 0-100: pinta una barra roja al pie de la portada, como en "Seguir viendo" de
   * cualquier plataforma real. Omitido cuando no aplica (no todo tiene progreso). */
  progressPercent?: number;
};

/** Tarjeta de portada 2:3. Reserva el espacio siempre (aspect-ratio fijo) para no saltar
 * el layout mientras carga la imagen; accesible por teclado (Enter abre, foco visible). */
export function PosterCard({ title, posterPath, subtitle, onOpen, gridIndex, favorite, progressPercent }: PosterCardProps) {
  const { data: base } = useApiBase();
  const src = imagePath(base, posterPath, "w342");

  return (
    <div className="group relative w-40 shrink-0 sm:w-44">
      <button
        type="button"
        data-grid-index={gridIndex}
        onClick={onOpen}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onOpen();
          }
        }}
        className="flex w-full flex-col gap-2 rounded text-left outline-none"
      >
        <div className="relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-neutral-800 ring-1 ring-white/5 transition-transform group-hover:scale-[1.03] group-focus-visible:scale-[1.03]">
          {src ? (
            <img src={src} alt="" loading="lazy" className="h-full w-full object-cover" />
          ) : (
            <div className="flex h-full w-full items-center justify-center p-3 text-center text-xs text-neutral-500">{title}</div>
          )}

          {progressPercent != null && (
            <div className="absolute inset-x-0 bottom-0 h-1 bg-black/50">
              <div className="h-full bg-red-600" style={{ width: `${Math.min(100, Math.max(0, progressPercent))}%` }} />
            </div>
          )}
        </div>
        <div>
          <p className="truncate text-sm font-medium text-neutral-100">{title}</p>
          {subtitle && <p className="truncate text-xs text-neutral-500">{subtitle}</p>}
        </div>
      </button>

      {favorite && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            favorite.onToggle();
          }}
          aria-label={favorite.isFavorite ? "Quitar de favoritos" : "Añadir a favoritos"}
          aria-pressed={favorite.isFavorite}
          className="absolute right-1.5 top-1.5 flex h-7 w-7 items-center justify-center rounded-full bg-black/60 text-white outline-none hover:bg-black/80 focus-visible:ring-2 focus-visible:ring-red-500"
        >
          <HeartIcon filled={favorite.isFavorite} className={`h-4 w-4 ${favorite.isFavorite ? "text-red-500" : ""}`} />
        </button>
      )}
    </div>
  );
}

export function PosterCardSkeleton() {
  return (
    <div className="flex w-40 shrink-0 flex-col gap-2 sm:w-44">
      <div className="aspect-[2/3] w-full animate-pulse rounded-lg bg-neutral-800" />
      <div className="h-4 w-3/4 animate-pulse rounded bg-neutral-800" />
    </div>
  );
}
