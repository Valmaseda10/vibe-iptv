import { useQuery } from "@tanstack/react-query";
import { SkeletonGrid } from "../../components/Skeleton";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { useFavorites } from "../../lib/favorites";
import { PosterCard } from "../../components/PosterCard";
import type { FavoriteItem } from "../../lib/api-types";
import { LargeTitle } from "../../components/LargeTitle";
import { ProfilePanel } from "./ProfilePanel";

/** «Mi lista» (antes «Favoritos»): lo que el perfil ha marcado con el corazón, una lista por
 * perfil (F "perfiles Netflix"). En el iPhone, arriba, el perfil con Ajustes, «Tu año»,
 * «Sorpréndeme» y «Cambiar de perfil» (`ProfilePanel`, 2026-11-02). */
export function FavoritesScreen() {
  const navigate = useNavigate();
  const { toggleFavorite } = useFavorites();
  const { profileId } = useActiveProfile();
  const { data, isPending } = useQuery({
    queryKey: ["favorites", profileId],
    queryFn: () => apiGet<FavoriteItem[]>(`/api/favorites?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const channels = data?.filter((f) => f.item_kind === "channel") ?? [];
  const movies = data?.filter((f) => f.item_kind === "movie") ?? [];
  const series = data?.filter((f) => f.item_kind === "series") ?? [];

  function open(item: FavoriteItem) {
    if (item.item_kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: String(item.item_id) } });
    else if (item.item_kind === "series") navigate({ to: "/series/$seriesId", params: { seriesId: String(item.item_id) } });
    else navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(item.item_id) } });
  }

  return (
    <div className="flex flex-col gap-6 px-4 py-5 sm:gap-8 sm:px-10 sm:py-8">
      <LargeTitle>Mi lista</LargeTitle>
      <ProfilePanel />

      {isPending && <SkeletonGrid count={12} />}
      {!isPending && channels.length === 0 && movies.length === 0 && series.length === 0 && (
        <p className="text-neutral-500">Tu lista está vacía. El corazón de cada película, serie o canal lo añade aquí.</p>
      )}

      {movies.length > 0 && (
        <section className="flex flex-col gap-3">
          <h2 className="text-lg font-semibold text-neutral-200">Películas</h2>
          <div className="flex flex-wrap gap-4">
            {movies.map((item) => (
              <PosterCard
                key={`movie-${item.item_id}`}
                title={item.title}
                posterPath={item.poster_path}
                onOpen={() => open(item)}
                favorite={{ isFavorite: true, onToggle: () => toggleFavorite("movie", item.item_id, false) }}
              />
            ))}
          </div>
        </section>
      )}

      {series.length > 0 && (
        <section className="flex flex-col gap-3">
          <h2 className="text-lg font-semibold text-neutral-200">Series</h2>
          <div className="flex flex-wrap gap-4">
            {series.map((item) => (
              <PosterCard
                key={`series-${item.item_id}`}
                title={item.title}
                posterPath={item.poster_path}
                onOpen={() => open(item)}
                favorite={{ isFavorite: true, onToggle: () => toggleFavorite("series", item.item_id, false) }}
              />
            ))}
          </div>
        </section>
      )}

      {channels.length > 0 && (
        <section className="flex flex-col gap-3">
          <h2 className="text-lg font-semibold text-neutral-200">Canales</h2>
          <ul className="flex flex-col gap-1">
            {channels.map((item) => (
              <li key={`channel-${item.item_id}`}>
                <button type="button" onClick={() => open(item)} className="w-full rounded-lg px-3 py-2 text-left outline-none hover:bg-neutral-900">
                  {item.title}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
