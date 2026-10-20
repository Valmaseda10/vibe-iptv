import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { PosterCard, PosterCardSkeleton } from "./PosterCard";
import { useFavorites } from "../lib/favorites";
import { recallScreenState, rememberScreenState } from "../lib/screenMemory";

export type GridItem = {
  id: number;
  title: string;
  posterPath: string | null;
  subtitle?: string;
  isNew?: boolean;
};

type VirtualPosterGridProps = {
  items: GridItem[];
  hasMore: boolean;
  isLoadingMore: boolean;
  onLoadMore: () => void;
  onOpen: (id: number) => void;
  emptyMessage: string;
  /** Presente activa el conmutador de favorito en cada tarjeta (F8); "movie" o "series". */
  favoriteKind?: string;
  /** La primerísima carga (todavía sin ninguna página en caché) es indistinguible de
   * "no hay nada" si solo se mira `items.length === 0` — sin esto se veía el mensaje de
   * vacío durante una fracción de segundo en cada entrada a la pantalla, antes de que
   * llegara la respuesta real. */
  isInitialLoading?: boolean;
  /** Si se da, recuerda el scroll de esta rejilla concreta (F "premium" — volver de una
   * ficha ya no manda de vuelta al principio del catálogo). Distinto por cada
   * combinación de filtros: cambiar de filtro debe empezar arriba, no en un scroll que
   * pertenecía a otra lista. Ver lib/screenMemory. */
  restoreKey?: string;
};

const CARD_WIDTH = 192; // 176px tarjeta (44 * 4) + 16px de separación
/** Por debajo de este ancho útil (iPhone en vertical, F15) la rejilla pasa a
 * `NARROW_COLUMNS` columnas que se reparten el ancho, como en cualquier app de móvil:
 * con tarjetas de ancho fijo cabría una sola. */
const NARROW_WIDTH = 600;
const NARROW_COLUMNS = 3;
/** Solo la estimación inicial: cada fila se mide de verdad al pintarse
 * (`measureElement`). Póster 2:3 de 176px (264) + título y año (~44) + separación (16).
 * Con una altura fija de 300 (la de la tarjeta estrecha de pantallas pequeñas), el año
 * quedaba tapado por la fila siguiente en cualquier ventana de escritorio. */
const ROW_HEIGHT_ESTIMATE = 330;

/** Rejilla virtualizada (TanStack Virtual): solo se montan las filas visibles, con un
 * margen (`overscan`). Las columnas por fila se recalculan con un `ResizeObserver` según
 * el ancho disponible, así que no hay un número de columnas fijo por breakpoint.
 *
 * Flechas para moverse entre tarjetas: como la rejilla está virtualizada, la tarjeta
 * destino puede no existir todavía en el DOM al pulsar la flecha, así que primero se
 * desplaza la fila con `scrollToIndex` y el foco se aplica en el siguiente pintado. */
export function VirtualPosterGrid({
  items,
  hasMore,
  isLoadingMore,
  onLoadMore,
  onOpen,
  emptyMessage,
  favoriteKind,
  isInitialLoading,
  restoreKey,
}: VirtualPosterGridProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState(4);
  const [fluid, setFluid] = useState(false);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const { isFavorite, toggleFavorite } = useFavorites();
  const restoredForKey = useRef<string | undefined>(undefined);

  // Mientras se ven el esqueleto de carga o el mensaje de vacío, `parentRef` no está
  // montado: se vuelve a enganchar al aparecer la rejilla (antes se quedaba en 4
  // columnas fijas para siempre si el primer render era el esqueleto).
  const showingGrid = !(isInitialLoading && items.length === 0) && !(items.length === 0 && !hasMore);
  useEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? 0;
      const narrow = width > 0 && width < NARROW_WIDTH;
      setFluid(narrow);
      setColumns(narrow ? NARROW_COLUMNS : Math.max(1, Math.floor(width / CARD_WIDTH)));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [showingGrid]);

  const loaderRow = hasMore ? 1 : 0;
  const rowCount = Math.ceil(items.length / columns) + loaderRow;

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT_ESTIMATE,
    overscan: 3,
  });

  const virtualRows = virtualizer.getVirtualItems();

  // Restaura el scroll guardado la primera vez que esta rejilla (identificada por
  // `restoreKey`) tiene datos que mostrar — antes de pintar, para que no se vea el
  // salto desde arriba. Solo una vez por key: los renders siguientes no deben pelearse
  // con el scroll que el usuario esté haciendo ahora mismo.
  useLayoutEffect(() => {
    if (!restoreKey || items.length === 0) return;
    if (restoredForKey.current === restoreKey) return;
    restoredForKey.current = restoreKey;
    const saved = recallScreenState<number>(`grid-scroll:${restoreKey}`);
    if (saved && parentRef.current) parentRef.current.scrollTop = saved;
  }, [restoreKey, items.length]);

  function onScroll() {
    if (!restoreKey || !parentRef.current) return;
    rememberScreenState(`grid-scroll:${restoreKey}`, parentRef.current.scrollTop);
  }

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (!last) return;
    if (hasMore && !isLoadingMore && last.index >= rowCount - 1) {
      onLoadMore();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.at(-1)?.index, hasMore, isLoadingMore, rowCount]);

  // Tras mover el foco lógico, la tarjeta puede tardar un pintado en aparecer (se acaba
  // de virtualizar su fila): se reintenta un par de frames antes de rendirse.
  useEffect(() => {
    if (focusedIndex == null) return;
    let frame = 0;
    let attempts = 0;
    const tryFocus = () => {
      const el = parentRef.current?.querySelector<HTMLElement>(`[data-grid-index="${focusedIndex}"]`);
      if (el) {
        el.focus();
        return;
      }
      attempts += 1;
      if (attempts < 6) frame = requestAnimationFrame(tryFocus);
    };
    frame = requestAnimationFrame(tryFocus);
    return () => cancelAnimationFrame(frame);
  }, [focusedIndex]);

  function onKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
    const currentAttr = (document.activeElement as HTMLElement | null)?.dataset.gridIndex;
    // La tarjeta enfocada de verdad manda sobre el foco lógico guardado: el usuario pudo
    // llegar a otra con el ratón o con Tab.
    const current = currentAttr ? Number(currentAttr) : (focusedIndex ?? 0);
    // Bordes de la rejilla (mando a distancia): arriba desde la primera fila sube a los
    // filtros, izquierda desde la primera columna vuelve a la barra lateral.
    if (e.key === "ArrowUp" && current < columns) {
      const filter = document.querySelector<HTMLElement>("[data-filter-bar] select, [data-filter-bar] input");
      if (filter) {
        e.preventDefault();
        filter.focus();
      }
      return;
    }
    if (e.key === "ArrowLeft" && current % columns === 0) {
      const navLink = document.querySelector<HTMLElement>('nav [aria-current="page"]') ?? document.querySelector<HTMLElement>("nav a");
      if (navLink) {
        e.preventDefault();
        navLink.focus();
      }
      return;
    }
    let next = current;
    if (e.key === "ArrowRight") next = current + 1;
    else if (e.key === "ArrowLeft") next = current - 1;
    else if (e.key === "ArrowDown") next = current + columns;
    else if (e.key === "ArrowUp") next = current - columns;
    next = Math.max(0, Math.min(items.length - 1, next));
    if (next === current) return;
    e.preventDefault();
    setFocusedIndex(next);
    virtualizer.scrollToIndex(Math.floor(next / columns), { align: "auto" });
  }

  if (isInitialLoading && items.length === 0) {
    return (
      <div className="flex flex-wrap gap-4 px-6 py-4 sm:px-10">
        {Array.from({ length: 8 }).map((_, i) => (
          <PosterCardSkeleton key={i} />
        ))}
      </div>
    );
  }

  if (items.length === 0 && !hasMore) {
    return <p className="px-6 py-10 text-neutral-500 sm:px-10">{emptyMessage}</p>;
  }

  return (
    <div ref={parentRef} onKeyDown={onKeyDown} onScroll={onScroll} className="h-full overflow-y-auto px-6 sm:px-10">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
        {virtualRows.map((virtualRow) => {
          const isLoaderRow = virtualRow.index === rowCount - 1 && hasMore;
          const startIndex = virtualRow.index * columns;
          const rowItems = isLoaderRow ? [] : items.slice(startIndex, startIndex + columns);

          return (
            <div
              key={virtualRow.key}
              ref={virtualizer.measureElement}
              data-index={virtualRow.index}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualRow.start}px)`,
                gridTemplateColumns: fluid ? `repeat(${columns}, minmax(0, 1fr))` : undefined,
              }}
              className={fluid ? "grid gap-3 pb-4" : "flex gap-4 pb-4"}
            >
              {isLoaderRow
                ? Array.from({ length: columns }).map((_, i) => <PosterCardSkeleton key={i} fluid={fluid} />)
                : rowItems.map((item, i) => (
                    <PosterCard
                      key={item.id}
                      gridIndex={startIndex + i}
                      title={item.title}
                      posterPath={item.posterPath}
                      subtitle={item.subtitle}
                      isNew={item.isNew}
                      fluid={fluid}
                      onOpen={() => onOpen(item.id)}
                      favorite={
                        favoriteKind
                          ? { isFavorite: isFavorite(favoriteKind, item.id), onToggle: () => toggleFavorite(favoriteKind, item.id, !isFavorite(favoriteKind, item.id)) }
                          : undefined
                      }
                    />
                  ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
