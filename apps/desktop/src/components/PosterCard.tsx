import { useApiBase, imagePath } from "../lib/api-client";

type PosterCardProps = {
  title: string;
  posterPath: string | null;
  subtitle?: string;
  onOpen: () => void;
  /** Índice dentro de una rejilla virtualizada (`VirtualPosterGrid`): permite localizar
   * y enfocar la tarjeta por teclado tras un `scrollToIndex`, ya que puede no existir
   * todavía en el DOM en el momento de pulsar la flecha. */
  gridIndex?: number;
};

/** Tarjeta de portada 2:3. Reserva el espacio siempre (aspect-ratio fijo) para no saltar
 * el layout mientras carga la imagen; accesible por teclado (Enter abre, foco visible). */
export function PosterCard({ title, posterPath, subtitle, onOpen, gridIndex }: PosterCardProps) {
  const { data: base } = useApiBase();
  const src = imagePath(base, posterPath, "w342");

  return (
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
      className="group flex w-40 shrink-0 flex-col gap-2 rounded text-left outline-none sm:w-44"
    >
      <div className="aspect-[2/3] w-full overflow-hidden rounded-lg bg-neutral-800 ring-1 ring-white/5 transition-transform group-hover:scale-[1.03] group-focus-visible:scale-[1.03]">
        {src ? (
          <img src={src} alt="" loading="lazy" className="h-full w-full object-cover" />
        ) : (
          <div className="flex h-full w-full items-center justify-center p-3 text-center text-xs text-neutral-500">{title}</div>
        )}
      </div>
      <div>
        <p className="truncate text-sm font-medium text-neutral-100">{title}</p>
        {subtitle && <p className="truncate text-xs text-neutral-500">{subtitle}</p>}
      </div>
    </button>
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
