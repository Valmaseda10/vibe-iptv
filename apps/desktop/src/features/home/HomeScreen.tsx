import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import { apiDelete, apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { FavoriteItem, HomeResponse, ProviderSummary } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { PosterCard, PosterCardSkeleton } from "../../components/PosterCard";
import { ProviderStatusPill } from "./ProviderStatusPill";
import { FavoriteChannelsRow } from "../../components/FavoriteChannelsRow";
import { BrowseScreen } from "../browse/BrowseScreen";

export function HomeScreen() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { profileId } = useActiveProfile();
  // Misma clave que Ajustes: al añadir el primer proveedor, Inicio se entera solo.
  const providersQuery = useQuery({
    queryKey: ["providers"],
    queryFn: () => invoke<ProviderSummary[]>("list_providers"),
  });
  const { data, isPending, isError } = useQuery({
    queryKey: ["home", profileId],
    queryFn: () => apiGet<HomeResponse>(`/api/home?profile_id=${profileId}`),
    enabled: profileId !== null,
  });
  // "Mi lista" (como Netflix): misma clave que los corazones, así se actualiza al instante.
  const favoritesQuery = useQuery({
    queryKey: ["favorites", profileId],
    queryFn: () => apiGet<FavoriteItem[]>(`/api/favorites?profile_id=${profileId}`),
    enabled: profileId !== null,
  });
  // Las 40 más recientes (el núcleo las da de la más nueva a la más vieja): una fila no
  // pinta cientos de portadas; la lista entera está en Favoritos.
  const myList = (favoritesQuery.data ?? []).filter((f) => (f.item_kind === "movie" || f.item_kind === "series") && f.title).slice(0, 40);

  // Optimista: la fila se siente al instante como Netflix, en vez de esperar la
  // respuesta del servidor para que el ítem desaparezca.
  const removeMutation = useMutation({
    mutationFn: (item: { item_kind: string; item_id: number }) =>
      apiDelete(`/api/playback/progress?profile_id=${profileId}&item_kind=${item.item_kind}&item_id=${item.item_id}`),
    onMutate: async (item) => {
      await queryClient.cancelQueries({ queryKey: ["home", profileId] });
      const previous = queryClient.getQueryData<HomeResponse>(["home", profileId]);
      queryClient.setQueryData<HomeResponse>(["home", profileId], (old) =>
        old ? { ...old, continue_watching: old.continue_watching.filter((c) => !(c.item_kind === item.item_kind && c.item_id === item.item_id)) } : old,
      );
      return { previous };
    },
    onError: (_err, _item, context) => {
      if (context?.previous) queryClient.setQueryData(["home", profileId], context.previous);
    },
  });

  function openContinueWatching(itemKind: string, itemId: number) {
    if (itemKind === "movie") navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(itemId) } });
    else if (itemKind === "episode") navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(itemId) } });
    else if (itemKind === "channel") navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(itemId) } });
  }

  function progressPercent(positionS: number, durationS: number | null): number | undefined {
    if (!durationS || durationS <= 0) return undefined;
    return (positionS / durationS) * 100;
  }

  if (providersQuery.data?.length === 0) {
    return <WelcomeWithoutProvider />;
  }

  // Lo personal va arriba, debajo de la portada; la portada y el resto de filas (una mezcla
  // de películas y series que cambia en cada visita) las da `GET /api/browse/home`.
  return (
    <BrowseScreen
      page="home"
      title="Inicio"
      actions={
        <div className="ml-auto">
          <ProviderStatusPill />
        </div>
      }
      before={
        <>
          {isError && <p className="px-4 text-red-400 sm:px-10">No se pudo cargar el inicio. ¿Hay algún proveedor sincronizado?</p>}

          {isPending ? (
            <SkeletonRow title="Seguir viendo" />
          ) : data && data.continue_watching.length > 0 ? (
            <Row title="Seguir viendo">
              {data.continue_watching.map((item) => (
                <PosterCard
                  key={`${item.item_kind}-${item.item_id}`}
                  title={item.title}
                  subtitle={item.subtitle ?? progressLabel(item.position_s, item.duration_s)}
                  posterPath={item.poster_path}
                  progressPercent={progressPercent(item.position_s, item.duration_s)}
                  onOpen={() => openContinueWatching(item.item_kind, item.item_id)}
                  onRemove={() => removeMutation.mutate({ item_kind: item.item_kind, item_id: item.item_id })}
                />
              ))}
            </Row>
          ) : null}

          {!isPending && data && data.next_episodes.length > 0 && (
            <Row title="Siguiente episodio">
              {data.next_episodes.map((item) => (
                <PosterCard
                  key={`next-${item.episode_id}`}
                  title={item.series_title}
                  subtitle={`T${item.season} E${item.number}${item.episode_title ? ` · ${item.episode_title}` : ""}`}
                  posterPath={item.still_path ?? item.poster_path}
                  onOpen={() => navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(item.episode_id) }, search: { seriesId: String(item.series_id) } })}
                />
              ))}
            </Row>
          )}

          <FavoriteChannelsRow />

          {myList.length > 0 && (
            <Row title="Mi lista">
              {myList.map((item) => (
                <PosterCard
                  key={`fav-${item.item_kind}-${item.item_id}`}
                  title={item.title}
                  posterPath={item.poster_path}
                  menuItem={{ kind: item.item_kind === "movie" ? "movie" : "series", id: item.item_id }}
                  onOpen={() =>
                    item.item_kind === "movie"
                      ? navigate({ to: "/movies/$movieId", params: { movieId: String(item.item_id) } })
                      : navigate({ to: "/series/$seriesId", params: { seriesId: String(item.item_id) } })
                  }
                />
              ))}
            </Row>
          )}
        </>
      }
    />
  );
}

/** Primera vez, sin ningún proveedor: en vez de filas vacías, decir qué hacer. */
function WelcomeWithoutProvider() {
  const navigate = useNavigate();
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-5 px-6 py-16 text-center">
      <h1 className="text-3xl font-semibold sm:text-4xl">Te damos la bienvenida a VIBE IPTV</h1>
      <p className="max-w-lg text-neutral-400">
        Añade tu proveedor IPTV (Xtream Codes o lista M3U) y aquí aparecerán tus canales, películas y series.
      </p>
      <button
        type="button"
        autoFocus
        onClick={() => navigate({ to: "/settings" })}
        className="rounded bg-red-600 px-6 py-3 font-medium outline-none hover:bg-red-500 focus-visible:ring-4 focus-visible:ring-red-400"
      >
        Añadir proveedor
      </button>
    </div>
  );
}

function progressLabel(positionS: number, durationS: number | null): string | undefined {
  if (!durationS || durationS <= 0) return undefined;
  const percent = Math.min(100, Math.round((positionS / durationS) * 100));
  return `${percent}% visto`;
}

function SkeletonRow({ title }: { title: string }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="px-4 text-lg font-semibold text-neutral-200 sm:px-10">{title}</h2>
      <div className="flex gap-4 overflow-hidden px-4 sm:px-10">
        {Array.from({ length: 6 }).map((_, i) => (
          <PosterCardSkeleton key={i} />
        ))}
      </div>
    </section>
  );
}
