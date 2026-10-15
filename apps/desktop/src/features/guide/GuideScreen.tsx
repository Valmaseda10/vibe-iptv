import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { CategorySummary, ChannelsPage, EpgGridProgramme } from "../../lib/api-types";
import { uniqueChannelsByName } from "../../lib/channels";

const WINDOW_HOURS = 4;
const WINDOW_SECONDS = WINDOW_HOURS * 3600;
const SHIFT_SECONDS = 3600;
const CHANNEL_LIMIT = 60; // una página razonable de canales por categoría; ver docs/DECISIONS.md.

function roundDownToHalfHour(epochSeconds: number): number {
  return Math.floor(epochSeconds / 1800) * 1800;
}

/** A qué ruta lleva pulsar una celda de programa: catch-up si ya terminó y el canal lo
 * soporta, directo en cualquier otro caso (en curso, futuro, o sin catch-up — el
 * backend decide si de verdad se puede, esto solo elige la ruta a intentar). */
function playerTargetFor(channel: ChannelsPage["items"][number], programme: EpgGridProgramme) {
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

/** Guía en rejilla (F7): filas = canales, columnas = tiempo. Flechas izquierda/derecha
 * desplazan la ventana temporal; arriba/abajo mueven el foco entre canales; Enter abre
 * el directo de la fila. Pulsar un programa ya emitido en un canal con catch-up (F9)
 * abre ese bloque concreto en vez del directo; el resto de celdas siguen abriendo el
 * directo (el backend rechaza con un mensaje claro si el proveedor no lo permite). */
export function GuideScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [windowStart, setWindowStart] = useState(() => roundDownToHalfHour(Date.now() / 1000));
  const [focusedIndex, setFocusedIndex] = useState(0);

  const categoriesQuery = useQuery({
    queryKey: ["guide-categories", profileId],
    queryFn: () => apiGet<CategorySummary[]>(`/api/live/categories?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const channelsQuery = useQuery({
    queryKey: ["guide-channels", profileId, categoryId],
    queryFn: () =>
      apiGet<ChannelsPage>(`/api/live/channels?profile_id=${profileId}&limit=${CHANNEL_LIMIT}${categoryId ? `&category_id=${categoryId}` : ""}`),
    enabled: profileId !== null,
  });
  const channels = useMemo(() => uniqueChannelsByName(channelsQuery.data?.items ?? []), [channelsQuery.data]);
  const channelIds = useMemo(() => channels.map((c) => c.id), [channels]);

  const windowEnd = windowStart + WINDOW_SECONDS;
  const gridQuery = useQuery({
    queryKey: ["guide-grid", channelIds.join(","), windowStart],
    queryFn: () => apiGet<EpgGridProgramme[]>(`/api/epg/grid?channel_ids=${channelIds.join(",")}&from=${windowStart}&to=${windowEnd}`),
    enabled: channelIds.length > 0,
  });

  const programmesByChannel = useMemo(() => {
    const map = new Map<number, EpgGridProgramme[]>();
    for (const p of gridQuery.data ?? []) {
      const list = map.get(p.channel_id) ?? [];
      list.push(p);
      map.set(p.channel_id, list);
    }
    return map;
  }, [gridQuery.data]);

  useEffect(() => {
    const el = document.querySelector<HTMLElement>(`[data-guide-row="${focusedIndex}"]`);
    el?.focus();
  }, [focusedIndex]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
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
          e.preventDefault();
          setFocusedIndex((i) => Math.max(0, i - 1));
          break;
        case "ArrowDown":
          e.preventDefault();
          setFocusedIndex((i) => Math.min(channels.length - 1, i + 1));
          break;
        case "Enter": {
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
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [channels, focusedIndex, navigate]);

  const hourMarks = Array.from({ length: WINDOW_HOURS }, (_, i) => windowStart + i * 3600);

  return (
    <div className="flex h-full flex-col gap-4 py-8">
      <div className="flex flex-wrap items-center justify-between gap-3 px-6 sm:px-10">
        <h1 className="text-3xl font-semibold">Guía</h1>
        <div className="flex items-center gap-3">
          <select
            className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
            value={categoryId ?? ""}
            onChange={(e) => {
              setCategoryId(e.target.value ? Number(e.target.value) : null);
              setFocusedIndex(0);
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
            className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900"
            aria-label="Retroceder una hora"
          >
            ← 1h
          </button>
          <button
            type="button"
            onClick={() => setWindowStart(roundDownToHalfHour(Date.now() / 1000))}
            className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900"
          >
            Ahora
          </button>
          <button
            type="button"
            onClick={() => setWindowStart((w) => w + SHIFT_SECONDS)}
            className="rounded border border-neutral-700 px-3 py-2 text-sm outline-none hover:bg-neutral-900"
            aria-label="Avanzar una hora"
          >
            1h →
          </button>
        </div>
      </div>

      {channels.length === 0 ? (
        <p className="px-6 text-neutral-500 sm:px-10">{channelsQuery.isPending ? "Cargando canales…" : "Sin canales en esta categoría."}</p>
      ) : (
        <div className="flex-1 overflow-auto px-6 sm:px-10">
          <div className="min-w-[900px]">
            <div className="mb-2 flex border-b border-neutral-800 pb-2 text-xs text-neutral-500">
              <div className="w-48 shrink-0" />
              <div className="flex flex-1">
                {hourMarks.map((ts) => (
                  <div key={ts} className="flex-1 border-l border-neutral-800 pl-2">
                    {formatHour(ts)}
                  </div>
                ))}
              </div>
            </div>

            <div className="flex flex-col gap-1">
              {channels.map((channel, index) => (
                <div
                  key={channel.id}
                  data-guide-row={index}
                  tabIndex={index === focusedIndex ? 0 : -1}
                  className={`flex h-14 items-center rounded-lg outline-none ${index === focusedIndex ? "ring-2 ring-red-600" : ""}`}
                >
                  <div className="flex w-48 shrink-0 items-center gap-2 pr-3">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-[9px] text-neutral-500">
                      {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
                    </div>
                    <span className="truncate text-sm">{channel.display_name}</span>
                  </div>

                  <div className="relative h-full flex-1 rounded bg-neutral-900">
                    {(programmesByChannel.get(channel.id) ?? []).map((p) => {
                      const left = Math.max(0, ((p.start_ts - windowStart) / WINDOW_SECONDS) * 100);
                      const right = Math.min(100, ((p.stop_ts - windowStart) / WINDOW_SECONDS) * 100);
                      if (right <= 0 || left >= 100) return null;
                      return (
                        <button
                          type="button"
                          key={`${p.start_ts}-${p.title}`}
                          title={p.title}
                          onClick={() => navigate(playerTargetFor(channel, p))}
                          className="absolute top-1 h-[calc(100%-8px)] overflow-hidden truncate rounded border border-neutral-700 bg-neutral-800 px-2 text-left text-xs outline-none hover:bg-neutral-700 focus-visible:ring-2 focus-visible:ring-red-600"
                          style={{ left: `${left}%`, width: `${right - left}%` }}
                        >
                          {p.title}
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))}
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
