import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useInfiniteQuery, useQueries, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { CategorySummary, ChannelsPage, EpgGridProgramme } from "../../lib/api-types";
import { uniqueChannelsByName } from "../../lib/channels";
import { useReminders } from "../../lib/reminders";
import { BellIcon } from "../../components/icons";
import { GuideSearchResults } from "./GuideSearchResults";
import { LargeTitle } from "../../components/LargeTitle";

const WINDOW_HOURS = 4;
const WINDOW_SECONDS = WINDOW_HOURS * 3600;
const SHIFT_SECONDS = 3600;
const ROW_HEIGHT = 60;
const CHANNEL_PAGE = 100;
/** La programación se pide por bloques de filas: una petición por bloque visible. */
const EPG_CHUNK = 25;

type Channel = ChannelsPage["items"][number];
type PageParam = { after: number; key: string } | undefined;

function roundDownToHalfHour(epochSeconds: number): number {
  return Math.floor(epochSeconds / 1800) * 1800;
}

/** A qué ruta lleva pulsar un programa ya emitido o en emisión: catch-up si ya terminó y
 * el canal lo ofrece, directo en cualquier otro caso. Los futuros ponen un recordatorio. */
function playerTargetFor(channel: Channel, programme: EpgGridProgramme) {
  const nowSeconds = Date.now() / 1000;
  const alreadyAired = programme.stop_ts <= nowSeconds;
  const categoryId = channel.category_id ? String(channel.category_id) : undefined;
  const catchup =
    alreadyAired && channel.has_catchup
      ? { catchupStart: programme.start_ts, catchupDuration: Math.max(1, Math.ceil((programme.stop_ts - programme.start_ts) / 60)) }
      : {};
  return {
    to: "/player/$kind/$id" as const,
    params: { kind: "channel", id: String(channel.id) },
    search: { categoryId, ...catchup },
  };
}

/** Guía en rejilla (F7, F13): filas = canales, columnas = tiempo. Todos los canales, sin
 * límite: se cargan por páginas y solo se pintan las filas visibles (R4), y la
 * programación se pide solo para esas. Flechas izquierda/derecha mueven la ventana de
 * tiempo; arriba/abajo, el canal; Enter abre el directo. Un programa futuro se pulsa para
 * que avise cuando empiece; uno emitido abre el catch-up si el canal lo ofrece. */
export function GuideScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [windowStart, setWindowStart] = useState(() => roundDownToHalfHour(Date.now() / 1000));
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [query, setQuery] = useState("");
  const searching = query.trim().length >= 2;
  const reminders = useReminders();

  const categoriesQuery = useQuery({
    queryKey: ["guide-categories", profileId],
    queryFn: () => apiGet<CategorySummary[]>(`/api/live/categories?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const channelsQuery = useInfiniteQuery({
    queryKey: ["guide-channels", profileId, categoryId],
    queryFn: ({ pageParam }: { pageParam: PageParam }) =>
      apiGet<ChannelsPage>(
        `/api/live/channels?profile_id=${profileId}&limit=${CHANNEL_PAGE}${categoryId ? `&category_id=${categoryId}` : ""}${
          pageParam ? `&after=${pageParam.after}&after_key=${encodeURIComponent(pageParam.key)}` : ""
        }`,
      ),
    initialPageParam: undefined as PageParam,
    getNextPageParam: (last) => (last.next_cursor != null ? { after: last.next_cursor, key: last.next_cursor_key ?? "" } : undefined),
    enabled: profileId !== null,
  });
  const channels = useMemo(() => uniqueChannelsByName((channelsQuery.data?.pages ?? []).flatMap((p) => p.items)), [channelsQuery.data]);
  const hasMore = channelsQuery.hasNextPage ?? false;

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: channels.length + (hasMore ? 1 : 0),
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 6,
  });
  const virtualRows = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (last && hasMore && !channelsQuery.isFetchingNextPage && last.index >= channels.length - 1) {
      channelsQuery.fetchNextPage();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.at(-1)?.index, hasMore, channelsQuery.isFetchingNextPage, channels.length]);

  // Programación de los bloques de filas visibles, cada uno en su propia caché: al
  // desplazarse solo se piden los bloques nuevos.
  const windowEnd = windowStart + WINDOW_SECONDS;
  const visibleChunks = useMemo(() => {
    const chunks = new Set<number>();
    for (const row of virtualRows) if (row.index < channels.length) chunks.add(Math.floor(row.index / EPG_CHUNK));
    return [...chunks].sort((a, b) => a - b);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows.map((r) => r.index).join(","), channels.length]);
  const gridQueries = useQueries({
    queries: visibleChunks.map((chunk) => {
      const ids = channels.slice(chunk * EPG_CHUNK, (chunk + 1) * EPG_CHUNK).map((c) => c.id);
      return {
        queryKey: ["guide-grid", ids.join(","), windowStart],
        queryFn: () => apiGet<EpgGridProgramme[]>(`/api/epg/grid?channel_ids=${ids.join(",")}&from=${windowStart}&to=${windowEnd}`),
        enabled: ids.length > 0,
        staleTime: 60_000,
      };
    }),
  });
  const programmesByChannel = useMemo(() => {
    const map = new Map<number, EpgGridProgramme[]>();
    for (const q of gridQueries) {
      for (const p of q.data ?? []) {
        const list = map.get(p.channel_id) ?? [];
        list.push(p);
        map.set(p.channel_id, list);
      }
    }
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gridQueries.map((q) => q.dataUpdatedAt).join(",")]);

  useEffect(() => {
    if (searching) return;
    let frame = 0;
    let attempts = 0;
    const tryFocus = () => {
      const el = parentRef.current?.querySelector<HTMLElement>(`[data-guide-row="${focusedIndex}"]`);
      if (el) {
        if (!el.contains(document.activeElement)) el.focus();
        return;
      }
      attempts += 1;
      if (attempts < 6) frame = requestAnimationFrame(tryFocus);
    };
    frame = requestAnimationFrame(tryFocus);
    return () => cancelAnimationFrame(frame);
  }, [focusedIndex, searching]);

  function onGridKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        setWindowStart((w) => w - SHIFT_SECONDS);
        break;
      case "ArrowRight":
        e.preventDefault();
        setWindowStart((w) => w + SHIFT_SECONDS);
        break;
      case "ArrowUp":
      case "ArrowDown": {
        e.preventDefault();
        const next = Math.max(0, Math.min(channels.length - 1, focusedIndex + (e.key === "ArrowDown" ? 1 : -1)));
        setFocusedIndex(next);
        virtualizer.scrollToIndex(next, { align: "auto" });
        break;
      }
      case "Enter": {
        // Enter sobre un programa lo maneja su botón; sobre la fila, abre el directo.
        if ((e.target as HTMLElement).dataset.guideRow === undefined) return;
        const channel = channels[focusedIndex];
        if (channel) {
          navigate({
            to: "/player/$kind/$id",
            params: { kind: "channel", id: String(channel.id) },
            search: channel.category_id ? { categoryId: String(channel.category_id) } : {},
          });
        }
        break;
      }
    }
  }

  const hourMarks = Array.from({ length: WINDOW_HOURS }, (_, i) => windowStart + i * 3600);
  const nowSeconds = Date.now() / 1000;
  const nowOffset = ((nowSeconds - windowStart) / WINDOW_SECONDS) * 100;

  return (
    <div className="flex h-full flex-col gap-4 py-5 sm:py-8">
      <div className="flex flex-wrap items-center justify-between gap-3 px-4 sm:px-10">
        <LargeTitle>Guía</LargeTitle>
        <div className="flex w-full flex-wrap items-center gap-2 sm:w-auto sm:gap-3">
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Buscar en la guía…"
            aria-label="Buscar en la guía"
            className="w-full rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none focus:border-neutral-500 sm:w-56"
          />
          {!searching && (
            <>
              <select
                className="min-w-0 flex-1 rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none sm:flex-none"
                value={categoryId ?? ""}
                aria-label="Categoría"
                onChange={(e) => {
                  setCategoryId(e.target.value ? Number(e.target.value) : null);
                  setFocusedIndex(0);
                  virtualizer.scrollToIndex(0);
                }}
              >
                <option value="">Todas las categorías</option>
                {categoriesQuery.data?.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
              <button
                type="button"
                onClick={() => setWindowStart((w) => w - SHIFT_SECONDS)}
                className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-red-600"
                aria-label="Retroceder una hora"
              >
                ← 1h
              </button>
              <button
                type="button"
                onClick={() => setWindowStart(roundDownToHalfHour(Date.now() / 1000))}
                className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-red-600"
              >
                Ahora
              </button>
              <button
                type="button"
                onClick={() => setWindowStart((w) => w + SHIFT_SECONDS)}
                className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-red-600"
                aria-label="Avanzar una hora"
              >
                1h →
              </button>
            </>
          )}
        </div>
      </div>

      {searching ? (
        <GuideSearchResults query={query.trim()} reminders={reminders} />
      ) : channels.length === 0 ? (
        <p className="px-4 text-neutral-500 sm:px-10">{channelsQuery.isPending ? "Cargando canales…" : "Sin canales en esta categoría."}</p>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col px-4 sm:px-10">
          <div className="mb-2 flex border-b border-neutral-800 pb-2 text-xs text-neutral-500">
            <div className="w-[5.5rem] shrink-0 sm:w-48" />
            <div className="flex flex-1">
              {hourMarks.map((ts) => (
                <div key={ts} className="flex-1 border-l border-neutral-800 pl-2">
                  {formatHour(ts)}
                </div>
              ))}
            </div>
          </div>

          <div ref={parentRef} className="min-h-0 flex-1 overflow-y-auto" onKeyDown={onGridKeyDown}>
            <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
              {virtualRows.map((row) => {
                const channel = channels[row.index];
                return (
                  <div key={row.key} className="absolute left-0 top-0 w-full" style={{ transform: `translateY(${row.start}px)`, height: ROW_HEIGHT }}>
                    {channel ? (
                      <div
                        data-guide-row={row.index}
                        tabIndex={row.index === focusedIndex ? 0 : -1}
                        onFocus={() => setFocusedIndex(row.index)}
                        className={`flex h-14 items-center rounded-lg outline-none ${row.index === focusedIndex ? "ring-2 ring-red-600" : ""}`}
                      >
                        <div className="flex w-[5.5rem] shrink-0 items-center gap-2 pr-3 sm:w-48">
                          {channel.number != null && <span className="w-8 shrink-0 text-right text-xs tabular-nums text-neutral-500">{channel.number}</span>}
                          <div className="flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[9px] text-neutral-500">
                            {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                          </div>
                          <span className="hidden truncate text-sm sm:inline">{channel.display_name}</span>
                        </div>

                        <div className="relative h-full flex-1 overflow-hidden rounded bg-neutral-900">
                          {nowOffset > 0 && nowOffset < 100 && (
                            <div className="pointer-events-none absolute inset-y-0 z-10 w-px bg-red-600/70" style={{ left: `${nowOffset}%` }} />
                          )}
                          {(programmesByChannel.get(channel.id) ?? []).map((p) => {
                            const left = Math.max(0, ((p.start_ts - windowStart) / WINDOW_SECONDS) * 100);
                            const right = Math.min(100, ((p.stop_ts - windowStart) / WINDOW_SECONDS) * 100);
                            if (right <= 0 || left >= 100) return null;
                            const future = p.start_ts > nowSeconds;
                            const reminded = reminders.has(channel.id, p.start_ts);
                            return (
                              <button
                                type="button"
                                key={`${p.start_ts}-${p.title}`}
                                title={future ? (reminded ? `${p.title} — quitar recordatorio` : `${p.title} — avísame cuando empiece`) : p.title}
                                onClick={() => {
                                  if (future) reminders.toggle(channel, p);
                                  else navigate(playerTargetFor(channel, p));
                                }}
                                className={`absolute top-1 flex h-[calc(100%-8px)] items-center gap-1 overflow-hidden rounded border px-2 text-left text-xs outline-none hover:bg-neutral-700 focus-visible:ring-2 focus-visible:ring-red-600 ${
                                  reminded ? "border-amber-500/70 bg-amber-500/10" : "border-neutral-700 bg-neutral-800"
                                }`}
                                style={{ left: `${left}%`, width: `${right - left}%` }}
                              >
                                {reminded && <BellIcon className="h-3 w-3 shrink-0 text-amber-400" aria-label="Con recordatorio" />}
                                <span className="truncate">{p.title}</span>
                              </button>
                            );
                          })}
                        </div>
                      </div>
                    ) : (
                      <div className="flex h-14 items-center px-2 text-sm text-neutral-500">Cargando más canales…</div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function formatHour(epochSeconds: number): string {
  const date = new Date(epochSeconds * 1000);
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}
