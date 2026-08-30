import { useEffect } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, useApiBase, imagePath } from "../../lib/api-client";
import type { MovieDetailResponse } from "../../lib/api-types";

export function MovieDetailScreen() {
  const { movieId } = useParams({ from: "/movies/$movieId" });
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const { data, isPending, isError } = useQuery({
    queryKey: ["movie", movieId],
    queryFn: () => apiGet<MovieDetailResponse>(`/api/movies/${movieId}`),
  });

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") navigate({ to: "/movies" });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate]);

  if (isPending) return <DetailSkeleton />;
  if (isError || !data) return <p className="p-10 text-red-400">No se encontró la película.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  const qualities = [...new Set(data.sources.map((s) => s.quality).filter((q): q is string => Boolean(q)))];

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
          {data.year && <span>{data.year}</span>}
          {data.runtime_min && <span>{data.runtime_min} min</span>}
          {data.rating != null && <span>★ {data.rating.toFixed(1)}</span>}
          <span className="rounded bg-neutral-800 px-2 py-0.5 text-xs uppercase tracking-wide text-neutral-400">{tmdbStateLabel(data.tmdb_state)}</span>
        </div>
        {data.overview && <p className="max-w-2xl text-neutral-300">{data.overview}</p>}

        <div className="flex gap-3 pt-2">
          <button
            type="button"
            disabled
            title="La reproducción llega en la fase F6"
            className="cursor-not-allowed rounded-lg bg-red-600/50 px-6 py-3 font-medium text-white/70"
          >
            ▶ Reproducir
          </button>
        </div>

        {data.sources.length > 0 && (
          <p className="pt-6 text-sm text-neutral-500">
            {data.sources.length} fuente{data.sources.length === 1 ? "" : "s"} disponible{data.sources.length === 1 ? "" : "s"}
            {qualities.length > 0 ? ` (${qualities.join(", ")})` : ""}
          </p>
        )}
      </div>
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
