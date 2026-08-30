import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import type { HomeResponse } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { PosterCard, PosterCardSkeleton } from "../../components/PosterCard";

export function HomeScreen() {
  const navigate = useNavigate();
  const { data, isPending, isError } = useQuery({
    queryKey: ["home"],
    queryFn: () => apiGet<HomeResponse>("/api/home"),
  });

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
              title={`${item.item_kind === "movie" ? "Película" : item.item_kind === "episode" ? "Episodio" : "Canal"} #${item.item_id}`}
              posterPath={null}
              onOpen={() => {
                if (item.item_kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: String(item.item_id) } });
              }}
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
