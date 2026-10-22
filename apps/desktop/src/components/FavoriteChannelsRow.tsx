import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../lib/api-client";
import { useActiveProfile } from "../lib/activeProfile";
import type { ChannelsPage } from "../lib/api-types";
import { Row } from "./Row";

/** Fila de canales favoritos (logo y nombre): un toque y se ve, con la lista y el zapping
 * del reproductor en los favoritos. No sale si no hay ninguno. */
export function FavoriteChannelsRow({ title = "★ Canales favoritos" }: { title?: string }) {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  // Misma consulta que la pantalla de Directo: se comparte la caché.
  const query = useQuery({
    queryKey: ["live-favorites", profileId],
    queryFn: () => apiGet<ChannelsPage>(`/api/live/channels?profile_id=${profileId}&limit=40&favorites=1`),
    enabled: profileId !== null,
  });
  const channels = query.data?.items ?? [];
  if (channels.length === 0) return null;

  return (
    <Row title={title}>
      {channels.map((channel) => (
        <button
          key={channel.id}
          type="button"
          data-card
          onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(channel.id) }, search: { favorites: true } })}
          className="flex shrink-0 items-center gap-3 self-start rounded-xl bg-neutral-900 py-2 pl-2 pr-4 text-sm outline-none transition-colors hover:bg-neutral-800 focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-full ios:bg-ios-surface-1 ios:text-[15px]"
        >
          <span className="flex h-10 w-10 items-center justify-center overflow-hidden rounded-lg bg-neutral-800 text-[10px] text-neutral-500 ios:rounded-full ios:bg-ios-surface-2">
            {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : "TV"}
          </span>
          <span className="max-w-44 truncate">{channel.display_name}</span>
        </button>
      ))}
    </Row>
  );
}
