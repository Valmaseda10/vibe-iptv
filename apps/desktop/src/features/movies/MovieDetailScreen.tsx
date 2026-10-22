import { useEffect, useState } from "react";
import { RatingControl } from "../../components/RatingControl";
import { useCanGoBack, useNavigate, useParams, useRouter } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiGet, apiGetOrNull, apiPost, errorMessage, useApiBase, imagePath } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { useFavorites } from "../../lib/favorites";
import { HeartIcon } from "../../components/icons";
import { TrailerModal } from "../../components/TrailerModal";
import { Row } from "../../components/Row";
import { PosterCard } from "../../components/PosterCard";
import type { MovieDetailResponse, PlaybackProgressResponse } from "../../lib/api-types";
import { DETAIL_ACTION, PRIMARY_ACTION } from "../../components/detailAction";
import { FloatingBackButton } from "../../components/FloatingBackButton";
import { BackdropTrailer } from "../../components/BackdropTrailer";

export function MovieDetailScreen() {
  const { movieId } = useParams({ from: "/movies/$movieId" });
  const navigate = useNavigate();
  const router = useRouter();
  const canGoBack = useCanGoBack();
  // Volver lleva a donde se estaba (Inicio, Buscar, Favoritos...), no siempre al listado;
  // sin historial (p. ej. entrando por "Sorpréndeme" nada más abrir), al listado.
  const goBack = () => (canGoBack ? router.history.back() : navigate({ to: "/movies" }));
  const { data: base } = useApiBase();
  const { profileId } = useActiveProfile();
  const { isFavorite, toggleFavorite } = useFavorites();
  const queryClient = useQueryClient();
  const [trailerOpen, setTrailerOpen] = useState(false);
  const [selectedSourceId, setSelectedSourceId] = useState<number | null>(null);
  // "Continuar" con su barra de progreso, como en Netflix (F15).
  const progressQuery = useQuery({
    queryKey: ["progress", "movie", movieId, profileId],
    queryFn: () => apiGetOrNull<PlaybackProgressResponse>(`/api/playback/progress?profile_id=${profileId}&item_kind=movie&item_id=${movieId}`),
    enabled: profileId !== null,
  });
  const resume = progressQuery.data && !progressQuery.data.finished && progressQuery.data.position_s > 5 ? progressQuery.data : null;
  const resumePercent = resume?.duration_s ? Math.min(100, (resume.position_s / resume.duration_s) * 100) : null;
  const remainingMin = resume?.duration_s ? Math.max(1, Math.round((resume.duration_s - resume.position_s) / 60)) : null;
  const { data, isPending, isError, error } = useQuery({
    queryKey: ["movie", movieId, profileId],
    queryFn: () => apiGet<MovieDetailResponse>(`/api/movies/${movieId}${profileId != null ? `?profile_id=${profileId}` : ""}`),
  });

  // Marcar como vista sin reproducir (F "premium", como Plex/Emby) — paridad con el
  // checkmark de episodio en la ficha de serie.
  const watchedMutation = useMutation({
    mutationFn: (nextWatched: boolean) => apiPost("/api/playback/movie-watched", { profile_id: profileId, movie_id: Number(movieId), watched: nextWatched }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["movie", movieId, profileId] }),
  });

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      // Con el tráiler abierto, Escape es cosa suya (cerrarlo) — si además volviera a
      // "Películas", el modal y la navegación se pisarían en la misma pulsación.
      if (e.key === "Escape" && !trailerOpen) goBack();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // `goBack` depende de `canGoBack`: sin él aquí, Escape usaría un valor viejo.
  }, [navigate, trailerOpen, canGoBack, router]);

  if (isPending) return <DetailSkeleton />;
  // 403: el perfil no puede ver este título (F14); el núcleo dice por qué.
  if (isError && error instanceof ApiError && error.status === 403) return <p className="p-10 text-neutral-300">{errorMessage(error)}</p>;
  if (isError || !data) return <p className="p-10 text-red-400">No se encontró la película.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  // Sin fondo de TMDB (sin clave, o título sin ficha), el póster hace de fondo difuminado
  // y aparece junto al título: mejor que un gran hueco negro.
  const poster = backdrop ? null : imagePath(base, data.poster_path, "w500");
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
      <div className="absolute inset-0 h-[60vh] overflow-hidden" style={{ ["--trailer-h" as string]: "60vh" }}>
        {backdrop && <img src={backdrop} alt="" className="h-full w-full object-cover" />}
        {backdrop && data.trailer_key && <BackdropTrailer youtubeKey={data.trailer_key} />}
        {poster && <img src={poster} alt="" className="h-full w-full scale-110 object-cover opacity-40 blur-2xl" />}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/70 to-transparent ios:from-black ios:via-black/60" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/90 via-neutral-950/10 to-transparent ios:from-black/70 ios:via-black/10" />
      </div>

      <div className={`relative flex flex-col gap-5 px-4 pb-16 sm:px-10 ${backdrop ? "pt-[32vh]" : "pt-10"}`}>
        <FloatingBackButton onClick={goBack} />
        <button type="button" onClick={goBack} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200 ios:hidden ios:desk:block">
          ← Volver <span className="pointer-coarse:hidden">(Esc)</span>
        </button>
        {poster && <img src={poster} alt="" className="w-36 rounded-lg shadow-2xl ring-1 ring-white/10 sm:w-44" />}
        <h1 className="max-w-2xl text-4xl font-bold sm:text-5xl">{data.clean_title}</h1>
        <div className="flex flex-wrap items-center gap-3 text-sm text-neutral-300">
          {data.age_rating && <span className="rounded border border-neutral-500 px-1.5 py-0.5 text-xs font-semibold">{data.age_rating}</span>}
          {data.year && <span>{data.year}</span>}
          {durationMin ? <span>{durationMin} min</span> : null}
          {data.rating != null && <span>★ {data.rating.toFixed(1)}</span>}
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

        <div className="flex flex-col gap-3 pt-2 sm:flex-row sm:items-start">
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() =>
                navigate({
                  to: "/player/$kind/$id",
                  params: { kind: "movie", id: movieId },
                  search: activeSourceId ? { source: activeSourceId } : {},
                })
              }
              // Con mando, abrir una ficha y pulsar OK debe reproducir: el foco llega aquí.
              autoFocus
              className={PRIMARY_ACTION}
            >
              ▶ {resume ? "Continuar" : "Reproducir"}
            </button>
            {resume && resumePercent != null && (
              <div className="flex items-center gap-2 text-xs text-neutral-400">
                <div className="h-1 flex-1 overflow-hidden rounded bg-neutral-700 sm:w-32 sm:flex-none">
                  <div className="h-full bg-red-600" style={{ width: `${resumePercent}%` }} />
                </div>
                {remainingMin != null && <span>Quedan {remainingMin} min</span>}
              </div>
            )}
          </div>
          <div className="grid auto-cols-fr grid-flow-col gap-1 sm:flex sm:gap-3">
            <button
              type="button"
              onClick={() => toggleFavorite("movie", data.id, !isFavorite("movie", data.id))}
              aria-pressed={isFavorite("movie", data.id)}
              className={DETAIL_ACTION}
            >
              <HeartIcon filled={isFavorite("movie", data.id)} className={`h-6 w-6 sm:h-5 sm:w-5 ${isFavorite("movie", data.id) ? "text-red-500" : ""}`} />
              {isFavorite("movie", data.id) ? "En mi lista" : "Mi lista"}
            </button>
            <button
              type="button"
              onClick={() => watchedMutation.mutate(!data.watched)}
              aria-pressed={data.watched}
              className={`${DETAIL_ACTION} ${data.watched ? "text-emerald-400 sm:border-emerald-700" : ""}`}
            >
              <span className="text-xl leading-none sm:text-base">{data.watched ? "✓" : "○"}</span>
              {data.watched ? "Vista" : "Marcar vista"}
            </button>
            <RatingControl kind="movie" id={data.id} title={data.clean_title} />
            {data.trailer_key && (
              <button type="button" onClick={() => setTrailerOpen(true)} className={DETAIL_ACTION}>
                <span className="text-xl leading-none sm:text-base">🎬</span>
                Tráiler
              </button>
            )}
          </div>
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

function DetailSkeleton() {
  return (
    <div className="flex flex-col gap-4 p-10">
      <div className="h-8 w-40 animate-pulse rounded bg-neutral-800" />
      <div className="h-12 w-96 animate-pulse rounded bg-neutral-800" />
      <div className="h-24 w-full max-w-2xl animate-pulse rounded bg-neutral-800" />
    </div>
  );
}
