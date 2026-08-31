import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useFavorites } from "../../lib/favorites";
import { PosterCard } from "../../components/PosterCard";
import type { FavoriteItem } from "../../lib/api-types";

/** Lista de favoritos (F8): sin esto, marcar algo como favorito no llevaba a ningún
 * sitio — el conmutador cambiaba, pero no había manera de volver a encontrarlo. */
export function FavoritesScreen() {
  const navigate = useNavigate();
  const { toggleFavorite } = useFavorites();
  const { data, isPending } = useQuery({
    queryKey: ["favorites"],
    queryFn: () => apiGet<FavoriteItem[]>("/api/favorites"),
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
    <div className="flex flex-col gap-8 px-6 py-8 sm:px-10">
      <h1 className="text-3xl font-semibold">Favoritos</h1>

      {isPending && <p className="text-neutral-500">Cargando…</p>}
      {!isPending && channels.length === 0 && movies.length === 0 && series.length === 0 && (
        <p className="text-neutral-500">Todavía no has marcado nada como favorito. El corazón de cada tarjeta lo añade aquí.</p>
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
