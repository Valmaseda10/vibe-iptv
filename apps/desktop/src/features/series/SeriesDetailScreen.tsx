import { useEffect, useMemo, useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, useApiBase, imagePath } from "../../lib/api-client";
import { useFavorites } from "../../lib/favorites";
import { HeartIcon } from "../../components/icons";
import type { EpisodeSummary, SeriesDetailResponse } from "../../lib/api-types";

export function SeriesDetailScreen() {
  const { seriesId } = useParams({ from: "/series/$seriesId" });
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const { isFavorite, toggleFavorite } = useFavorites();
  const { data, isPending, isError } = useQuery({
    queryKey: ["series", seriesId],
    // series_detail() puede llamarse bajo demanda en el núcleo (primera visita o caché
    // de más de 7 días): esta petición puede tardar más que el resto de la API.
    queryFn: () => apiGet<SeriesDetailResponse>(`/api/series/${seriesId}`),
  });

  const seasons = useMemo(() => groupBySeason(data?.episodes ?? []), [data]);
  const [selectedSeason, setSelectedSeason] = useState<number | null>(null);
  const activeSeason = selectedSeason ?? seasons[0]?.season ?? null;

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") navigate({ to: "/series" });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate]);

  if (isPending) return <DetailSkeleton />;
  if (isError || !data) return <p className="p-10 text-red-400">No se encontró la serie.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  const episodesOfSeason = seasons.find((s) => s.season === activeSeason)?.episodes ?? [];

  return (
    <div className="relative min-h-full">
      <div className="absolute inset-0 h-[50vh] overflow-hidden">
        {backdrop && <img src={backdrop} alt="" className="h-full w-full object-cover" />}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/70 to-transparent" />
      </div>

      <div className="relative flex flex-col gap-5 px-6 pb-16 pt-[26vh] sm:px-10">
        <button type="button" onClick={() => navigate({ to: "/series" })} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200">
          ← Volver (Esc)
        </button>
        <h1 className="max-w-2xl text-4xl font-bold sm:text-5xl">{data.clean_title}</h1>
        <div className="flex flex-wrap items-center gap-3 text-sm text-neutral-300">
          {data.year && <span>{data.year}</span>}
          {data.rating != null && <span>★ {data.rating.toFixed(1)}</span>}
        </div>
        {data.overview && <p className="max-w-2xl text-neutral-300">{data.overview}</p>}

        <div className="flex items-center gap-3 pt-2">
          <button
            type="button"
            onClick={() => toggleFavorite("series", data.id, !isFavorite("series", data.id))}
            aria-pressed={isFavorite("series", data.id)}
            className="flex items-center gap-2 rounded-lg border border-neutral-700 px-4 py-3 text-sm outline-none hover:bg-neutral-900"
          >
            <HeartIcon filled={isFavorite("series", data.id)} className={`h-5 w-5 ${isFavorite("series", data.id) ? "text-red-500" : ""}`} />
            {isFavorite("series", data.id) ? "En favoritos" : "Añadir a favoritos"}
          </button>
        </div>

        {seasons.length === 0 ? (
          <p className="pt-6 text-sm text-neutral-500">
            Todavía no hay episodios descargados de esta serie. Se piden al proveedor la primera vez que se abre la ficha.
          </p>
        ) : (
          <div className="pt-6">
            <div className="mb-4 flex gap-2 overflow-x-auto" role="tablist" aria-label="Temporadas">
              {seasons.map((s) => (
                <button
                  key={s.season}
                  role="tab"
                  aria-selected={s.season === activeSeason}
                  onClick={() => setSelectedSeason(s.season)}
                  className={`shrink-0 rounded-full px-4 py-2 text-sm outline-none ${
                    s.season === activeSeason ? "bg-red-600 text-white" : "bg-neutral-800 text-neutral-300 hover:bg-neutral-700"
                  }`}
                >
                  Temporada {s.season}
                </button>
              ))}
            </div>

            <ul className="flex flex-col gap-3">
              {episodesOfSeason.map((episode) => (
                <li key={episode.id}>
                  <button
                    type="button"
                    onClick={() =>
                      navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(episode.id) }, search: { seriesId } })
                    }
                    className="flex w-full gap-4 rounded-lg p-2 text-left outline-none hover:bg-neutral-900 focus-visible:bg-neutral-900"
                  >
                    <div className="flex h-20 w-32 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-xs text-neutral-500">
                      {imagePath(base, episode.still_path, "w342") ? (
                        <img src={imagePath(base, episode.still_path, "w342")!} alt="" className="h-full w-full object-cover" />
                      ) : (
                        episode.number
                      )}
                    </div>
                    <div className="min-w-0">
                      <p className="truncate font-medium">
                        {episode.number}. {episode.title || `Episodio ${episode.number}`}
                      </p>
                      {episode.duration_s && <p className="text-xs text-neutral-500">{Math.round(episode.duration_s / 60)} min</p>}
                      {episode.overview && <p className="mt-1 line-clamp-2 text-sm text-neutral-400">{episode.overview}</p>}
                    </div>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}

function groupBySeason(episodes: EpisodeSummary[]) {
  const bySeason = new Map<number, EpisodeSummary[]>();
  for (const ep of episodes) {
    const list = bySeason.get(ep.season) ?? [];
    list.push(ep);
    bySeason.set(ep.season, list);
  }
  return [...bySeason.entries()]
    .sort(([a], [b]) => a - b)
    .map(([season, eps]) => ({ season, episodes: eps.sort((a, b) => a.number - b.number) }));
}

function DetailSkeleton() {
  return (
    <div className="flex flex-col gap-4 p-10">
      <div className="h-8 w-40 animate-pulse rounded bg-neutral-800" />
      <div className="h-12 w-96 animate-pulse rounded bg-neutral-800" />
      <div className="h-24 w-full max-w-2xl animate-pulse rounded bg-neutral-800" />
    </div>
  );
}
