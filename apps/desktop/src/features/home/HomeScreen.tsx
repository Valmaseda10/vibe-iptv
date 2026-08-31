import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { HomeResponse } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { PosterCard, PosterCardSkeleton } from "../../components/PosterCard";

export function HomeScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const { data, isPending, isError } = useQuery({
    queryKey: ["home", profileId],
    queryFn: () => apiGet<HomeResponse>(`/api/home?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  function openContinueWatching(itemKind: string, itemId: number) {
    if (itemKind === "movie") navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(itemId) } });
    else if (itemKind === "episode") navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(itemId) } });
    else if (itemKind === "channel") navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(itemId) } });
  }

  return (
    <div className="flex flex-col gap-10 py-8">
      <h1 className="px-6 text-3xl font-semibold sm:px-10">Inicio</h1>

      {isError && <p className="px-6 text-red-400 sm:px-10">No se pudo cargar el inicio. ¿Hay algún proveedor sincronizado?</p>}

      {isPending ? (
        <SkeletonRow title="Continuar viendo" />
      ) : data && data.continue_watching.length > 0 ? (
        <Row title="Continuar viendo">
          {data.continue_watching.map((item) => (
            <PosterCard
              key={`${item.item_kind}-${item.item_id}`}
              title={item.title}
              subtitle={item.subtitle ?? progressLabel(item.position_s, item.duration_s)}
              posterPath={item.poster_path}
              onOpen={() => openContinueWatching(item.item_kind, item.item_id)}
            />
          ))}
        </Row>
      ) : null}

      {isPending ? (
        <SkeletonRow title="Siguiente episodio" />
      ) : data && data.next_episodes.length > 0 ? (
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
      ) : null}

      {isPending ? (
        <SkeletonRow title="Recientes" />
      ) : (
        <Row title="Recientes">
          {(data?.recent_movies ?? []).map((movie) => (
            <PosterCard
              key={movie.id}
              title={movie.clean_title}
              posterPath={movie.poster_path}
              subtitle={movie.year ? String(movie.year) : undefined}
              onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(movie.id) } })}
            />
          ))}
          {data && data.recent_movies.length === 0 && <p className="text-neutral-500">Todavía no hay películas sincronizadas.</p>}
        </Row>
      )}
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
      <h2 className="px-6 text-lg font-semibold text-neutral-200 sm:px-10">{title}</h2>
      <div className="flex gap-4 overflow-hidden px-6 sm:px-10">
        {Array.from({ length: 6 }).map((_, i) => (
          <PosterCardSkeleton key={i} />
        ))}
      </div>
    </section>
  );
}
