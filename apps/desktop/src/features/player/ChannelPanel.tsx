import { useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { apiGet } from "../../lib/api-client";
import type { CategorySummary, ChannelsPage, EpgNowNextResponse } from "../../lib/api-types";
import { groupChannelsByName } from "../../lib/channels";
import { CloseIcon } from "../../components/icons";
import type { PlayerSearch } from "../../lib/playerSession";

const ROW_HEIGHT = 64;
const FAVORITES_SCOPE = "fav";

type ChannelPanelProps = {
  profileId: number;
  /** Canal que se está viendo: se marca y la lista arranca en él si está cargado. */
  currentChannelId: number;
  /** Nombre del canal que se ve: marca también sus otras fuentes. */
  currentName: string | undefined;
  /** Lista con la que se abrió el reproductor (favoritos o una categoría). */
  initialScope: { categoryId?: string; favorites?: boolean };
  onPick: (channelId: number, search: PlayerSearch) => void;
  onClose: () => void;
};

/** Lista de canales dentro del reproductor del PC, a la derecha y encima del vídeo, para
 * cambiar sin salir. Con selector de categoría (todas, «★ Favoritos» o cualquiera). Un
 * canal con varias fuentes es una sola fila (abre la primera; la fuente se cambia abajo
 * en el reproductor). Flechas para moverse, Enter para ver y Esc para cerrar. */
export function ChannelPanel({ profileId, currentChannelId, currentName, initialScope, onPick, onClose }: ChannelPanelProps) {
  const [scope, setScope] = useState(initialScope.favorites ? FAVORITES_SCOPE : (initialScope.categoryId ?? ""));
  const favoritesOnly = scope === FAVORITES_SCOPE;
  const categoryId = !favoritesOnly && scope ? Number(scope) : null;

  const categoriesQuery = useQuery({
    queryKey: ["live-categories", profileId],
    queryFn: () => apiGet<CategorySummary[]>(`/api/live/categories?profile_id=${profileId}`),
  });
  // Misma consulta (y misma caché) que la pantalla de Directo.
  const channelsQuery = useInfiniteQuery({
    queryKey: ["live-channels", profileId, categoryId, favoritesOnly],
    queryFn: ({ pageParam }: { pageParam: { after: number; key: string } | undefined }) =>
      apiGet<ChannelsPage>(
        `/api/live/channels?profile_id=${profileId}&limit=80${favoritesOnly ? "&favorites=1" : categoryId ? `&category_id=${categoryId}` : ""}${
          pageParam ? `&after=${pageParam.after}&after_key=${encodeURIComponent(pageParam.key)}` : ""
        }`,
      ),
    initialPageParam: undefined as { after: number; key: string } | undefined,
    getNextPageParam: (lastPage) => (lastPage.next_cursor != null ? { after: lastPage.next_cursor, key: lastPage.next_cursor_key ?? "" } : undefined),
  });
  const items = useMemo(() => (channelsQuery.data?.pages ?? []).flatMap((page) => page.items), [channelsQuery.data]);
  const groups = useMemo(() => groupChannelsByName(items), [items]);
  const hasMore = channelsQuery.hasNextPage ?? false;

  const channelIds = items.map((c) => c.id).join(",");
  const epgQuery = useQuery({
    queryKey: ["live-epg-now", channelIds],
    queryFn: () => apiGet<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${channelIds}`),
    enabled: channelIds.length > 0,
    refetchInterval: 60_000,
  });
  const epgByChannel = useMemo(() => new Map((epgQuery.data ?? []).map((e) => [e.channel_id, e])), [epgQuery.data]);

  const listRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: groups.length + (hasMore ? 1 : 0),
    getScrollElement: () => listRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  });
  const virtualRows = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (last && hasMore && !channelsQuery.isFetchingNextPage && last.index >= groups.length - 1) channelsQuery.fetchNextPage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.at(-1)?.index, hasMore, channelsQuery.isFetchingNextPage, groups.length]);

  const isCurrent = (index: number) => {
    const group = groups[index];
    return Boolean(group && (group.displayName === currentName || group.channels.some((c) => c.id === currentChannelId)));
  };

  // Al abrir (y al cambiar de categoría) se enfoca el canal que se ve, o el primero.
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const initialFocusDone = useRef<string | null>(null);
  useEffect(() => {
    if (channelsQuery.isPending || initialFocusDone.current === scope) return;
    initialFocusDone.current = scope;
    const current = groups.findIndex((_, index) => isCurrent(index));
    const target = current >= 0 ? current : 0;
    if (groups.length === 0) return;
    // Un fotograma después: la lista virtual tiene que haber medido su altura antes.
    requestAnimationFrame(() => virtualizer.scrollToIndex(target, { align: "center" }));
    setFocusedIndex(target);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelsQuery.isPending, scope, groups.length]);

  // La fila puede no estar aún en el DOM (lista virtual): se reintenta unos fotogramas.
  useEffect(() => {
    if (focusedIndex == null) return;
    let frame = 0;
    let attempts = 0;
    const tryFocus = () => {
      const el = listRef.current?.querySelector<HTMLElement>(`[data-row-index="${focusedIndex}"]`);
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

  function onListKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown" && e.key !== "PageUp" && e.key !== "PageDown") return;
    e.preventDefault();
    const currentAttr = (document.activeElement as HTMLElement | null)?.dataset.rowIndex;
    const from = currentAttr ? Number(currentAttr) : (focusedIndex ?? 0);
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : e.key === "PageDown" ? 8 : -8;
    const next = Math.max(0, Math.min(groups.length - 1, from + step));
    // Si la fila ya está en el DOM, el foco pasa al instante (un Enter justo después
    // abre esa); si no, cuando la lista virtual la pinte.
    listRef.current?.querySelector<HTMLElement>(`[data-row-index="${next}"]`)?.focus();
    setFocusedIndex(next);
    virtualizer.scrollToIndex(next, { align: "auto" });
  }

  function pick(channelId: number) {
    onPick(channelId, favoritesOnly ? { favorites: true } : categoryId ? { categoryId: String(categoryId) } : {});
  }

  return (
    <aside
      className="absolute inset-y-0 right-0 z-20 flex w-[min(24rem,100%)] flex-col bg-neutral-950/95 shadow-2xl shadow-black backdrop-blur-sm"
      aria-label="Lista de canales"
      onMouseMove={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center gap-2 border-b border-white/10 px-4 py-3">
        <h2 className="flex-1 text-sm font-semibold text-white">Canales</h2>
        <button
          type="button"
          onClick={onClose}
          className="rounded-full p-1.5 text-neutral-400 outline-none hover:bg-white/10 hover:text-white focus-visible:ring-2 focus-visible:ring-white/60"
          aria-label="Cerrar la lista de canales (Esc)"
          title="Cerrar (Esc)"
        >
          <CloseIcon className="h-5 w-5" />
        </button>
      </div>
      <div className="px-4 py-3">
        <select
          aria-label="Categoría"
          value={scope}
          onChange={(e) => setScope(e.target.value)}
          className="w-full rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm text-neutral-100 outline-none focus:ring-2 focus:ring-white/60"
        >
          <option value="">Todas las categorías</option>
          <option value={FAVORITES_SCOPE}>★ Favoritos</option>
          {categoriesQuery.data?.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>
      <div ref={listRef} onKeyDown={onListKeyDown} className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {channelsQuery.isPending ? (
          <div className="flex flex-col gap-2 px-2 py-2">
            {Array.from({ length: 8 }).map((_, i) => (
              <div key={i} className="h-12 animate-pulse rounded bg-neutral-800" />
            ))}
          </div>
        ) : groups.length === 0 && !hasMore ? (
          <p className="px-3 py-8 text-sm text-neutral-500">{favoritesOnly ? "Aún no tienes canales favoritos." : "Sin canales en esta categoría."}</p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
            {virtualRows.map((row) => {
              const group = groups[row.index];
              const channel = group?.channels[0];
              const current = isCurrent(row.index);
              const now = channel ? epgByChannel.get(channel.id)?.current : undefined;
              return (
                <div key={row.key} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: row.size, transform: `translateY(${row.start}px)` }}>
                  {group && channel ? (
                    <button
                      type="button"
                      data-row-index={row.index}
                      onClick={() => pick(group.channels.find((c) => c.id === currentChannelId)?.id ?? channel.id)}
                      aria-current={current ? "true" : undefined}
                      className={`flex h-full w-full items-center gap-3 rounded-lg px-2 text-left outline-none hover:bg-white/10 focus-visible:bg-white/15 focus-visible:ring-2 focus-visible:ring-white/60 ${
                        current ? "bg-red-600/20" : ""
                      }`}
                    >
                      <span className="w-7 shrink-0 text-right text-xs tabular-nums text-neutral-500">{channel.number ?? ""}</span>
                      <span className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[10px] text-neutral-500">
                        {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className={`block truncate text-sm ${current ? "font-semibold text-white" : "text-neutral-100"}`}>{group.displayName}</span>
                        <span className="block truncate text-xs text-neutral-500">{now?.title ?? (group.channels.length > 1 ? `${group.channels.length} fuentes` : "")}</span>
                      </span>
                      {current && (
                        <span className="shrink-0 rounded bg-red-600 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white">Viendo</span>
                      )}
                    </button>
                  ) : (
                    <div className="mx-2 mt-2 h-12 animate-pulse rounded bg-neutral-800" />
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </aside>
  );
}
