import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { PosterCard, PosterCardSkeleton } from "./PosterCard";

export type GridItem = {
  id: number;
  title: string;
  posterPath: string | null;
  subtitle?: string;
};

type VirtualPosterGridProps = {
  items: GridItem[];
  hasMore: boolean;
  isLoadingMore: boolean;
  onLoadMore: () => void;
  onOpen: (id: number) => void;
  emptyMessage: string;
};

const CARD_WIDTH = 192; // 176px tarjeta (44 * 4) + 16px de separación
const ROW_HEIGHT = 300;

/** Rejilla virtualizada (TanStack Virtual): solo se montan las filas visibles, con un
 * margen (`overscan`). Las columnas por fila se recalculan con un `ResizeObserver` según
 * el ancho disponible, así que no hay un número de columnas fijo por breakpoint.
 *
 * Flechas para moverse entre tarjetas: como la rejilla está virtualizada, la tarjeta
 * destino puede no existir todavía en el DOM al pulsar la flecha, así que primero se
 * desplaza la fila con `scrollToIndex` y el foco se aplica en el siguiente pintado. */
export function VirtualPosterGrid({ items, hasMore, isLoadingMore, onLoadMore, onOpen, emptyMessage }: VirtualPosterGridProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState(4);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);

  useEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? 0;
      setColumns(Math.max(1, Math.floor(width / CARD_WIDTH)));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const loaderRow = hasMore ? 1 : 0;
  const rowCount = Math.ceil(items.length / columns) + loaderRow;

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 3,
  });

  const virtualRows = virtualizer.getVirtualItems();

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
    const current = focusedIndex ?? (currentAttr ? Number(currentAttr) : 0);
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

  if (items.length === 0 && !hasMore) {
    return <p className="px-6 py-10 text-neutral-500 sm:px-10">{emptyMessage}</p>;
  }

  return (
    <div ref={parentRef} onKeyDown={onKeyDown} className="h-full overflow-y-auto px-6 sm:px-10">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
        {virtualRows.map((virtualRow) => {
          const isLoaderRow = virtualRow.index === rowCount - 1 && hasMore;
          const startIndex = virtualRow.index * columns;
          const rowItems = isLoaderRow ? [] : items.slice(startIndex, startIndex + columns);

          return (
            <div
              key={virtualRow.key}
              style={{ position: "absolute", top: 0, left: 0, width: "100%", height: virtualRow.size, transform: `translateY(${virtualRow.start}px)` }}
              className="flex gap-4 pb-4"
            >
              {isLoaderRow
                ? Array.from({ length: columns }).map((_, i) => <PosterCardSkeleton key={i} />)
                : rowItems.map((item, i) => (
                    <PosterCard
                      key={item.id}
                      gridIndex={startIndex + i}
                      title={item.title}
                      posterPath={item.posterPath}
                      subtitle={item.subtitle}
                      onOpen={() => onOpen(item.id)}
                    />
                  ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
