import { useEffect, useMemo, useRef, useState } from "react";
import { HorizontalScroller } from "../../components/HorizontalScroller";
import { useNavigate } from "@tanstack/react-router";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { CategorySummary, ChannelsPage, EpgNowNextResponse } from "../../lib/api-types";
import { feedLabels, groupChannelsByName, type ChannelGroup } from "../../lib/channels";
import { useFavorites } from "../../lib/favorites";
import { HeartIcon } from "../../components/icons";
import { LargeTitle } from "../../components/LargeTitle";

const ROW_HEIGHT = 60;
const EPG_REFRESH_MS = 60_000;

type ChannelItem = ChannelsPage["items"][number];

const LIVE_SCOPE_KEY = "vibe.live.scope";
const FAVORITES_SCOPE = "fav";

function readSavedScope(): string {
  try {
    return window.localStorage.getItem(LIVE_SCOPE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function LiveScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  // Qué lista se ve: todas, «★ Favoritos» o una categoría. Se recuerda en este aparato: quien
  // ve sobre todo sus favoritos los tiene al entrar.
  const [scope, setScopeState] = useState<string>(() => readSavedScope());
  const setScope = (next: string) => {
    setScopeState(next);
    try {
      window.localStorage.setItem(LIVE_SCOPE_KEY, next);
    } catch {
      // Sin almacenamiento: solo no se recuerda.
    }
  };
  const favoritesOnly = scope === FAVORITES_SCOPE;
  const categoryId = !favoritesOnly && scope ? Number(scope) : null;
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const categoriesQuery = useQuery({
    queryKey: ["live-categories", profileId],
    queryFn: () => apiGet<CategorySummary[]>(`/api/live/categories?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

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
    enabled: profileId !== null,
  });
  const favoritesQuery = useQuery({
    queryKey: ["live-favorites", profileId],
    queryFn: () => apiGet<ChannelsPage>(`/api/live/channels?profile_id=${profileId}&limit=40&favorites=1`),
    enabled: profileId !== null,
  });
  const recentQuery = useQuery({
    queryKey: ["live-recent", profileId],
    queryFn: () => apiGet<ChannelItem[]>(`/api/live/recent?profile_id=${profileId}&limit=12`),
    enabled: profileId !== null,
  });

  const items = (channelsQuery.data?.pages ?? []).flatMap((page) => page.items);
  const hasMore = channelsQuery.hasNextPage ?? false;
  const groups = useMemo(() => groupChannelsByName(items), [items]);

  // Fuente elegida a mano por grupo (nombre de canal), si el usuario cambió del feed por
  // defecto (el primero). Por nombre y no por índice de grupo: así sobrevive a que
  // lleguen más páginas y el grupo se mueva de posición.
  const [selectedFeed, setSelectedFeed] = useState<Record<string, number>>({});
  function activeChannelFor(group: ChannelGroup): ChannelItem {
    const chosenId = selectedFeed[group.displayName];
    // `group.channels` nunca está vacío (groupChannelsByName solo crea un grupo al
    // meter el primer canal dentro): el `!` es seguro, no un valor real que pueda faltar.
    return group.channels.find((c) => c.id === chosenId) ?? group.channels[0]!;
  }

  // "Qué están dando" por canal (F7): se pide para los ya cargados, no para todo el
  // catálogo. Se refresca cada minuto para que el programa "actual" no se quede
  // desfasado mientras la pantalla está abierta.
  const channelIds = items.map((c) => c.id);
  const epgQuery = useQuery({
    queryKey: ["live-epg-now", channelIds.join(",")],
    queryFn: () => apiGet<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${channelIds.join(",")}`),
    enabled: channelIds.length > 0,
    refetchInterval: EPG_REFRESH_MS,
  });
  const epgByChannel = new Map((epgQuery.data ?? []).map((e) => [e.channel_id, e]));

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: groups.length + (hasMore ? 1 : 0),
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  });
  const virtualRows = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (last && hasMore && !channelsQuery.isFetchingNextPage && last.index >= groups.length - 1) {
      channelsQuery.fetchNextPage();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.at(-1)?.index, hasMore, channelsQuery.isFetchingNextPage, groups.length]);

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
    const next = Math.max(0, Math.min(groups.length - 1, current + (e.key === "ArrowDown" ? 1 : -1)));
    if (next === current) return;
    e.preventDefault();
    setFocusedIndex(next);
    virtualizer.scrollToIndex(next, { align: "auto" });
  }

  return (
    <div className="flex h-full flex-col gap-4 py-5 sm:py-8">
      <div className="flex flex-wrap items-center justify-between gap-3 px-4 sm:px-10">
        <LargeTitle>Directo</LargeTitle>
        <select
          aria-label="Categoría"
          className="w-full rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none sm:w-auto ios:rounded-full ios:border-0 ios:bg-white/10 ios:px-4 ios:py-2.5 ios:text-[15px]"
          value={scope}
          onChange={(e) => setScope(e.target.value)}
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

      {scope === "" && (favoritesQuery.data?.items.length ?? 0) > 0 && (
        <section className="px-4 sm:px-10" aria-label="Canales favoritos">
          <h2 className="mb-2 text-sm font-medium text-neutral-400 ios:text-[20px] ios:font-bold ios:tracking-tight ios:text-white">★ Favoritos</h2>
          <HorizontalScroller className="flex gap-2 overflow-x-auto pb-1">
            {favoritesQuery.data!.items.map((channel) => (
              <button
                key={channel.id}
                type="button"
                onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(channel.id) }, search: { favorites: true } })}
                className="flex shrink-0 items-center gap-2 rounded-lg bg-neutral-900 px-3 py-2 text-sm outline-none hover:bg-neutral-800 focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-full ios:bg-ios-surface-1 ios:py-1.5 ios:pl-1.5 ios:pr-4 ios:text-[15px]"
              >
                <span className="flex h-7 w-7 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[9px] text-neutral-500 ios:rounded-full ios:bg-ios-surface-2">
                  {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                </span>
                <span className="max-w-40 truncate">{channel.display_name}</span>
              </button>
            ))}
          </HorizontalScroller>
        </section>
      )}

      {scope === "" && (recentQuery.data?.length ?? 0) > 0 && (
        <section className="px-4 sm:px-10" aria-label="Canales recientes">
          <h2 className="mb-2 text-sm font-medium text-neutral-400 ios:text-[20px] ios:font-bold ios:tracking-tight ios:text-white">Recientes</h2>
          <HorizontalScroller className="flex gap-2 overflow-x-auto pb-1">
            {recentQuery.data!.map((channel) => (
              <button
                key={channel.id}
                type="button"
                onClick={() =>
                  navigate({
                    to: "/player/$kind/$id",
                    params: { kind: "channel", id: String(channel.id) },
                    search: channel.category_id ? { categoryId: String(channel.category_id) } : {},
                  })
                }
                className="flex shrink-0 items-center gap-2 rounded-lg bg-neutral-900 px-3 py-2 text-sm outline-none hover:bg-neutral-800 focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-full ios:bg-ios-surface-1 ios:py-1.5 ios:pl-1.5 ios:pr-4 ios:text-[15px]"
              >
                <span className="flex h-7 w-7 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[9px] text-neutral-500 ios:rounded-full ios:bg-ios-surface-2">
                  {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                </span>
                <span className="max-w-40 truncate">{channel.display_name}</span>
              </button>
            ))}
          </HorizontalScroller>
        </section>
      )}

      <div ref={parentRef} onKeyDown={onKeyDown} className="min-h-0 flex-1 overflow-y-auto px-4 sm:px-10">
        {channelsQuery.isPending ? (
          <div className="flex flex-col gap-2 py-4">
            {Array.from({ length: 10 }).map((_, i) => (
              <div key={i} className="h-10 animate-pulse rounded bg-neutral-800" />
            ))}
          </div>
        ) : groups.length === 0 && !hasMore ? (
          <p className="py-10 text-neutral-500">
            {favoritesOnly ? "Aún no tienes canales favoritos: toca el corazón de un canal para tenerlo aquí." : "Sin canales en esta categoría."}
          </p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
            {virtualRows.map((row) => {
              const group = groups[row.index];
              const channel = group ? activeChannelFor(group) : undefined;
              return (
                <div
                  key={row.key}
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", height: row.size, transform: `translateY(${row.start}px)` }}
                >
                  {group && channel ? (
                    <div className="flex h-full w-full items-center gap-2">
                      <button
                        type="button"
                        data-row-index={row.index}
                        onClick={() =>
                          navigate({
                            to: "/player/$kind/$id",
                            params: { kind: "channel", id: String(channel.id) },
                            search: favoritesOnly ? { favorites: true } : channel.category_id ? { categoryId: String(channel.category_id) } : {},
                          })
                        }
                        className="flex h-full min-w-0 flex-1 items-center gap-4 rounded-lg px-3 text-left outline-none hover:bg-neutral-900 ios:gap-3 ios:rounded-2xl ios:px-2 ios:active:bg-white/10"
                      >
                        <span className="w-8 shrink-0 text-right text-xs tabular-nums text-neutral-500">{channel.number ?? ""}</span>
                        <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[10px] text-neutral-500 ios:rounded-[10px] ios:bg-ios-surface-2">
                          {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                        </div>
                        <div className="min-w-0 flex-1">
                          <span className="block truncate text-sm ios:text-[17px] ios:text-white">{group.displayName}</span>
                          <NowPlaying epg={epgByChannel.get(channel.id)} />
                        </div>
                        {channel.has_catchup && <span className="ml-auto shrink-0 rounded bg-neutral-800 px-2 py-0.5 text-[10px] text-neutral-400">catch-up</span>}
                      </button>
                      <ChannelFavoriteToggle group={group} activeChannelId={channel.id} />
                      {group.channels.length > 1 && (
                        <select
                          value={channel.id}
                          onClick={(e) => e.stopPropagation()}
                          onChange={(e) => setSelectedFeed((prev) => ({ ...prev, [group.displayName]: Number(e.target.value) }))}
                          className="max-w-[6.5rem] shrink-0 truncate rounded bg-neutral-800 px-2 py-1 text-xs text-neutral-300 outline-none pointer-coarse:py-2 ios:rounded-full ios:bg-white/10 ios:px-3"
                          aria-label={`Elegir fuente para ${group.displayName}`}
                          title={`${group.channels.length} fuentes disponibles para este canal`}
                        >
                          {feedLabels(group.channels).map((label, i) => (
                            <option key={group.channels[i]!.id} value={group.channels[i]!.id}>
                              {label}
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
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

/** Programa actual con una barra de progreso fina, o nada si todavía no hay guía para
 * ese canal (proveedor sin XMLTV, o sincronización de EPG aún no ha corrido). */
function NowPlaying({ epg }: { epg: EpgNowNextResponse | undefined }) {
  const current = epg?.current;
  if (!current) return null;

  const nowSeconds = Date.now() / 1000;
  const total = current.stop_ts - current.start_ts;
  const elapsed = nowSeconds - current.start_ts;
  const progress = total > 0 ? Math.min(1, Math.max(0, elapsed / total)) : 0;

  return (
    <div className="mt-1 flex items-center gap-2">
      <span className="truncate text-xs text-neutral-500 ios:text-[13px] ios:text-ios-label-2">{current.title}</span>
      <div className="h-1 w-16 shrink-0 overflow-hidden rounded-full bg-neutral-800 ios:bg-white/15">
        <div className="h-full bg-red-600" style={{ width: `${progress * 100}%` }} />
      </div>
    </div>
  );
}

/** Corazón de cada fila de Directo. Un canal con varias fuentes es un solo canal para el
 * usuario: está en favoritos si lo está cualquiera de sus fuentes, y quitarlo las quita
 * todas; añadirlo marca la fuente que se está usando. */
function ChannelFavoriteToggle({ group, activeChannelId }: { group: ChannelGroup; activeChannelId: number }) {
  const { isFavorite, toggleFavorite } = useFavorites();
  const favoriteFeeds = group.channels.filter((c) => isFavorite("channel", c.id));
  const active = favoriteFeeds.length > 0;
  return (
    <button
      type="button"
      onClick={() => {
        if (active) favoriteFeeds.forEach((c) => toggleFavorite("channel", c.id, false));
        else toggleFavorite("channel", activeChannelId, true);
      }}
      aria-label={active ? `Quitar ${group.displayName} de favoritos` : `Añadir ${group.displayName} a favoritos`}
      aria-pressed={active}
      className="shrink-0 rounded-full p-2 text-neutral-400 outline-none hover:bg-neutral-800 hover:text-white pointer-coarse:p-3"
    >
      <HeartIcon filled={active} className={`h-4 w-4 ${active ? "text-red-500" : ""}`} />
    </button>
  );
}
