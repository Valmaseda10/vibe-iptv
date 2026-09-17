import { useEffect, useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, useApiBase, imagePath } from "../../lib/api-client";
import { useFavorites } from "../../lib/favorites";
import { HeartIcon } from "../../components/icons";
import { TrailerModal } from "../../components/TrailerModal";
import { Row } from "../../components/Row";
import { PosterCard } from "../../components/PosterCard";
import type { MovieDetailResponse } from "../../lib/api-types";

export function MovieDetailScreen() {
  const { movieId } = useParams({ from: "/movies/$movieId" });
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const { isFavorite, toggleFavorite } = useFavorites();
  const [trailerOpen, setTrailerOpen] = useState(false);
  const [selectedSourceId, setSelectedSourceId] = useState<number | null>(null);
  const { data, isPending, isError } = useQuery({
    queryKey: ["movie", movieId],
    queryFn: () => apiGet<MovieDetailResponse>(`/api/movies/${movieId}`),
  });

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      // Con el tráiler abierto, Escape es cosa suya (cerrarlo) — si además volviera a
      // "Películas", el modal y la navegación se pisarían en la misma pulsación.
      if (e.key === "Escape" && !trailerOpen) navigate({ to: "/movies" });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate, trailerOpen]);

  if (isPending) return <DetailSkeleton />;
  if (isError || !data) return <p className="p-10 text-red-400">No se encontró la película.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  const qualities = [...new Set(data.sources.map((s) => s.quality).filter((q): q is string => Boolean(q)))];
  // TMDB no siempre tiene la duración (títulos nuevos u oscuros); la duración real del
  // fichero, sin abrir ningún stream (F "premium", Source::vod_info), es un buen
  // respaldo cuando falta.
  const technicalDurationMin = data.sources.find((s) => s.duration_s)?.duration_s;
  const durationMin = data.runtime_min || (technicalDurationMin ? Math.round(technicalDurationMin / 60) : null);
  // Distintas calidades del mismo título pueden ser ficheros/proveedores distintos de
  // verdad (F "premium" — el backend acepta `?source=` desde F6, sin selector todavía
  // en la interfaz que lo usara). Sin elegir nada, el backend sigue usando la primera.
  const activeSourceId = selectedSourceId ?? data.sources[0]?.id ?? null;

  return (
    <div className="relative min-h-full">
      <div className="absolute inset-0 h-[60vh] overflow-hidden">
        {backdrop && <img src={backdrop} alt="" className="h-full w-full object-cover" />}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/70 to-transparent" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/90 via-neutral-950/10 to-transparent" />
      </div>

      <div className="relative flex flex-col gap-5 px-6 pb-16 pt-[32vh] sm:px-10">
        <button type="button" onClick={() => navigate({ to: "/movies" })} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200">
          ← Volver (Esc)
        </button>
        <h1 className="max-w-2xl text-4xl font-bold sm:text-5xl">{data.clean_title}</h1>
        <div className="flex flex-wrap items-center gap-3 text-sm text-neutral-300">
          {data.age_rating && <span className="rounded border border-neutral-500 px-1.5 py-0.5 text-xs font-semibold">{data.age_rating}</span>}
          {data.year && <span>{data.year}</span>}
          {durationMin ? <span>{durationMin} min</span> : null}
          {data.rating != null && <span>★ {data.rating.toFixed(1)}</span>}
          <span className="rounded bg-neutral-800 px-2 py-0.5 text-xs uppercase tracking-wide text-neutral-400">{tmdbStateLabel(data.tmdb_state)}</span>
        </div>
        {data.overview && <p className="max-w-2xl text-neutral-300">{data.overview}</p>}

        {data.sources.length > 1 && (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm text-neutral-500">Fuente:</span>
            {data.sources.map((source, i) => (
              <button
                key={source.id}
                type="button"
                aria-pressed={activeSourceId === source.id}
                onClick={() => setSelectedSourceId(source.id)}
                className={`rounded-full border px-3 py-1 text-xs outline-none ${
                  activeSourceId === source.id
                    ? "border-red-600 bg-red-600/20 text-white"
                    : "border-neutral-700 text-neutral-400 hover:border-neutral-500"
                }`}
              >
                {source.quality ?? `Fuente ${i + 1}`}
                {source.language ? ` · ${source.language}` : ""}
              </button>
            ))}
          </div>
        )}

        <div className="flex items-center gap-3 pt-2">
          <button
            type="button"
            onClick={() =>
              navigate({
                to: "/player/$kind/$id",
                params: { kind: "movie", id: movieId },
                search: activeSourceId ? { source: activeSourceId } : {},
              })
            }
            className="rounded-lg bg-red-600 px-6 py-3 font-medium text-white outline-none hover:bg-red-500"
          >
            ▶ Reproducir
          </button>
          <button
            type="button"
            onClick={() => toggleFavorite("movie", data.id, !isFavorite("movie", data.id))}
            aria-pressed={isFavorite("movie", data.id)}
            className="flex items-center gap-2 rounded-lg border border-neutral-700 px-4 py-3 text-sm outline-none hover:bg-neutral-900"
          >
            <HeartIcon filled={isFavorite("movie", data.id)} className={`h-5 w-5 ${isFavorite("movie", data.id) ? "text-red-500" : ""}`} />
            {isFavorite("movie", data.id) ? "En favoritos" : "Añadir a favoritos"}
          </button>
          {data.trailer_key && (
            <button
              type="button"
              onClick={() => setTrailerOpen(true)}
              className="flex items-center gap-2 rounded-lg border border-neutral-700 px-4 py-3 text-sm outline-none hover:bg-neutral-900"
            >
              🎬 Tráiler
            </button>
          )}
        </div>

        {trailerOpen && data.trailer_key && (
          <TrailerModal youtubeKey={data.trailer_key} title={data.clean_title} onClose={() => setTrailerOpen(false)} />
        )}

        {data.sources.length === 1 && (
          <p className="pt-6 text-sm text-neutral-500">1 fuente disponible{qualities.length > 0 ? ` (${qualities.join(", ")})` : ""}</p>
        )}
      </div>

      {data.similar.length > 0 && (
        <div className="relative pb-10">
          <Row title="Más como esto">
            {data.similar.map((title) => (
              <PosterCard
                key={title.id}
                title={title.clean_title}
                posterPath={title.poster_path}
                subtitle={title.year ? String(title.year) : undefined}
                onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(title.id) } })}
              />
            ))}
          </Row>
        </div>
      )}
    </div>
  );
}

function tmdbStateLabel(state: string) {
  switch (state) {
    case "matched":
      return "TMDB";
    case "pending":
      return "pendiente de TMDB";
    case "not_found":
      return "sin coincidencia en TMDB";
    default:
      return state;
  }
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
