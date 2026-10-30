import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../lib/api-client";
import { useActiveProfile } from "../lib/activeProfile";
import type { ChannelsPage, EpgNowNextResponse } from "../lib/api-types";
import { uniqueChannelsByName } from "../lib/channels";
import { Row } from "./Row";

const EPG_REFRESH_MS = 60_000;

function clock(epoch: number): string {
  return new Date(epoch * 1000).toLocaleTimeString("es-ES", { hour: "2-digit", minute: "2-digit" });
}

/** «Tus canales, ahora»: los canales favoritos con lo que están echando (guía), su hora,
 * cuánto lleva y lo siguiente. Un toque y se ve, con la lista y el zapping del reproductor
 * en los favoritos. No sale si no hay ninguno. */
export function FavoriteChannelsRow({ title = "Tus canales, ahora" }: { title?: string }) {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  // Misma consulta que la pantalla de Directo: se comparte la caché.
  const query = useQuery({
    queryKey: ["live-favorites", profileId],
    queryFn: () => apiGet<ChannelsPage>(`/api/live/channels?profile_id=${profileId}&limit=40&favorites=1`),
    enabled: profileId !== null,
  });
  // Varias fuentes del mismo canal en favoritos son un solo canal.
  const channels = uniqueChannelsByName(query.data?.items ?? []);
  const ids = channels.map((c) => c.id).join(",");
  const epgQuery = useQuery({
    queryKey: ["live-epg-now", ids],
    queryFn: () => apiGet<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${ids}`),
    enabled: ids.length > 0,
    refetchInterval: EPG_REFRESH_MS,
  });
  if (channels.length === 0) return null;
  const epgByChannel = new Map((epgQuery.data ?? []).map((e) => [e.channel_id, e]));
  const now = Date.now() / 1000;

  return (
    <Row title={title}>
      {channels.map((channel) => {
        const epg = epgByChannel.get(channel.id);
        const current = epg?.current;
        const total = current ? current.stop_ts - current.start_ts : 0;
        const progress = current && total > 0 ? Math.min(1, Math.max(0, (now - current.start_ts) / total)) : null;
        return (
          <button
            key={channel.id}
            type="button"
            data-card
            onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(channel.id) }, search: { favorites: true } })}
            className="flex w-64 shrink-0 flex-col gap-2 self-start rounded-xl bg-neutral-900 p-3 text-left outline-none transition-colors hover:bg-neutral-800 focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-2xl ios:bg-ios-surface-1 sm:w-72"
          >
            <span className="flex items-center gap-3">
              <span className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-neutral-800 text-[10px] text-neutral-500 ios:bg-ios-surface-2">
                {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-semibold text-white">{channel.display_name}</span>
                <span className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-red-500">
                  <span className="h-1.5 w-1.5 rounded-full bg-red-500" />
                  En directo
                </span>
              </span>
            </span>
            {current ? (
              <span className="flex flex-col gap-1.5">
                <span className="line-clamp-2 min-h-[2.5rem] text-sm text-neutral-100">{current.title}</span>
                <span className="flex items-center gap-2 text-[11px] tabular-nums text-neutral-400">
                  {clock(current.start_ts)}
                  <span className="h-1 flex-1 overflow-hidden rounded-full bg-white/15">
                    <span className="block h-full rounded-full bg-red-600" style={{ width: `${(progress ?? 0) * 100}%` }} />
                  </span>
                  {clock(current.stop_ts)}
                </span>
                <span className="truncate text-xs text-neutral-500">{epg?.next ? `Después: ${epg.next.title}` : " "}</span>
              </span>
            ) : (
              <span className="min-h-[2.5rem] text-sm text-neutral-500">Sin guía para este canal</span>
            )}
          </button>
        );
      })}
    </Row>
  );
}
