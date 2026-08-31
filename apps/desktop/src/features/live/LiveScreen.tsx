import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { apiGet } from "../../lib/api-client";
import type { CategorySummary, ChannelsPage } from "../../lib/api-types";

const ROW_HEIGHT = 60;

export function LiveScreen() {
  const navigate = useNavigate();
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const categoriesQuery = useQuery({
    queryKey: ["live-categories"],
    queryFn: () => apiGet<CategorySummary[]>("/api/live/categories"),
  });

  const channelsQuery = useInfiniteQuery({
    queryKey: ["live-channels", categoryId],
    queryFn: ({ pageParam }: { pageParam: number | undefined }) =>
      apiGet<ChannelsPage>(`/api/live/channels?limit=80${categoryId ? `&category_id=${categoryId}` : ""}${pageParam ? `&after=${pageParam}` : ""}`),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
  });

  const items = (channelsQuery.data?.pages ?? []).flatMap((page) => page.items);
  const hasMore = channelsQuery.hasNextPage ?? false;

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length + (hasMore ? 1 : 0),
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  });
  const virtualRows = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (last && hasMore && !channelsQuery.isFetchingNextPage && last.index >= items.length - 1) {
      channelsQuery.fetchNextPage();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.at(-1)?.index, hasMore, channelsQuery.isFetchingNextPage, items.length]);

  // Igual que en VirtualPosterGrid: la fila destino puede no existir aún en el DOM al
  // pulsar la flecha (está virtualizada), así que se reintenta unos frames.
  useEffect(() => {
    if (focusedIndex == null) return;
    let frame = 0;
    let attempts = 0;
    const tryFocus = () => {
      const el = parentRef.current?.querySelector<HTMLElement>(`[data-row-index="${focusedIndex}"]`);
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
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    const currentAttr = (document.activeElement as HTMLElement | null)?.dataset.rowIndex;
    const current = focusedIndex ?? (currentAttr ? Number(currentAttr) : 0);
    const next = Math.max(0, Math.min(items.length - 1, current + (e.key === "ArrowDown" ? 1 : -1)));
    if (next === current) return;
    e.preventDefault();
    setFocusedIndex(next);
    virtualizer.scrollToIndex(next, { align: "auto" });
  }

  return (
    <div className="flex h-full flex-col gap-4 py-8">
      <div className="flex flex-wrap items-center justify-between gap-3 px-6 sm:px-10">
        <h1 className="text-3xl font-semibold">Directo</h1>
        <select
          className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
          value={categoryId ?? ""}
          onChange={(e) => setCategoryId(e.target.value ? Number(e.target.value) : null)}
        >
          <option value="">Todas las categorías</option>
          {categoriesQuery.data?.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>

      <div ref={parentRef} onKeyDown={onKeyDown} className="min-h-0 flex-1 overflow-y-auto px-6 sm:px-10">
        {items.length === 0 && !hasMore ? (
          <p className="py-10 text-neutral-500">Sin canales en esta categoría.</p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
            {virtualRows.map((row) => {
              const channel = items[row.index];
              return (
                <div
                  key={row.key}
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", height: row.size, transform: `translateY(${row.start}px)` }}
                >
                  {channel ? (
                    <button
                      type="button"
                      data-row-index={row.index}
                      onClick={() =>
                        navigate({
                          to: "/player/$kind/$id",
                          params: { kind: "channel", id: String(channel.id) },
                          search: channel.category_id ? { categoryId: String(channel.category_id) } : {},
                        })
                      }
                      className="flex h-full w-full items-center gap-4 rounded-lg px-3 text-left outline-none hover:bg-neutral-900"
                    >
                      <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[10px] text-neutral-500">
                        {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                      </div>
                      <span className="truncate text-sm">{channel.display_name}</span>
                      {channel.has_catchup && <span className="ml-auto shrink-0 rounded bg-neutral-800 px-2 py-0.5 text-[10px] text-neutral-400">catch-up</span>}
                    </button>
                  ) : (
                    <div className="mx-3 h-10 animate-pulse rounded bg-neutral-800" />
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
