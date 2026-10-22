import { useEffect, useMemo, useState } from "react";
import { RatingControl } from "../../components/RatingControl";
import { useCanGoBack, useNavigate, useParams, useRouter } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiGet, apiPost, errorMessage, useApiBase, imagePath } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { useFavorites } from "../../lib/favorites";
import { HeartIcon } from "../../components/icons";
import { TrailerModal } from "../../components/TrailerModal";
import { DETAIL_ACTION, PRIMARY_ACTION } from "../../components/detailAction";
import { FloatingBackButton } from "../../components/FloatingBackButton";
import { BackdropTrailer } from "../../components/BackdropTrailer";
import { Row } from "../../components/Row";
import { PosterCard } from "../../components/PosterCard";
import type { EpisodeSummary, SeriesDetailResponse } from "../../lib/api-types";

export function SeriesDetailScreen() {
  const { seriesId } = useParams({ from: "/series/$seriesId" });
  const navigate = useNavigate();
  const router = useRouter();
  const canGoBack = useCanGoBack();
  // Volver lleva a donde se estaba (Inicio, Buscar, Favoritos...), no siempre al listado;
  // sin historial (p. ej. entrando por "Sorpréndeme" nada más abrir), al listado.
  const goBack = () => (canGoBack ? router.history.back() : navigate({ to: "/series" }));
  const { data: base } = useApiBase();
  const { profileId } = useActiveProfile();
  const { isFavorite, toggleFavorite } = useFavorites();
  const queryClient = useQueryClient();
  const [trailerOpen, setTrailerOpen] = useState(false);
  const { data, isPending, isError, error } = useQuery({
    queryKey: ["series", seriesId, profileId],
    // series_detail() puede llamarse bajo demanda en el núcleo (primera visita o caché
    // de más de 7 días): esta petición puede tardar más que el resto de la API.
    queryFn: () => apiGet<SeriesDetailResponse>(`/api/series/${seriesId}?profile_id=${profileId}`),
    // La ficha llega al instante con lo guardado; lo que faltaba (tráiler, clasificación,
    // duración, episodios nuevos) se pide por detrás: se vuelve a preguntar unas veces.
    refetchInterval: (query) => (query.state.data?.pending_details && query.state.dataUpdateCount < 4 ? 2500 : false),
    enabled: profileId !== null,
  });

  function invalidateSeries() {
    queryClient.invalidateQueries({ queryKey: ["series", seriesId, profileId] });
  }

  // Marcar/desmarcar sin reproducir (F "premium", como Plex/Emby) — antes el checkmark
  // era de solo lectura, solo reflejaba lo que se había visto de verdad.
  const episodeWatchedMutation = useMutation({
    mutationFn: (input: { episodeId: number; watched: boolean }) =>
      apiPost("/api/playback/episode-watched", { profile_id: profileId, episode_id: input.episodeId, watched: input.watched }),
    onSuccess: invalidateSeries,
  });
  const seasonWatchedMutation = useMutation({
    mutationFn: (input: { season: number; watched: boolean }) =>
      apiPost("/api/playback/season-watched", { profile_id: profileId, series_id: Number(seriesId), season: input.season, watched: input.watched }),
    onSuccess: invalidateSeries,
  });

  const seasons = useMemo(() => groupBySeason(data?.episodes ?? []), [data]);
  const [selectedSeason, setSelectedSeason] = useState<number | null>(null);
  const activeSeason = selectedSeason ?? seasons[0]?.season ?? null;

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && !trailerOpen) goBack();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // `goBack` depende de `canGoBack`: sin él aquí, Escape usaría un valor viejo.
  }, [navigate, trailerOpen, canGoBack, router]);

  if (isPending) return <DetailSkeleton />;
  // 403: el perfil no puede ver este título (F14); el núcleo dice por qué.
  if (isError && error instanceof ApiError && error.status === 403) return <p className="p-10 text-neutral-300">{errorMessage(error)}</p>;
  if (isError || !data) return <p className="p-10 text-red-400">No se encontró la serie.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  // Mismo criterio que la ficha de película: sin fondo de TMDB, el póster ocupa su lugar.
  const poster = backdrop ? null : imagePath(base, data.poster_path, "w500");
  const episodesOfSeason = seasons.find((s) => s.season === activeSeason)?.episodes ?? [];
  const seasonWatched = episodesOfSeason.length > 0 && episodesOfSeason.every((e) => e.watched);
  // "Ver/Continuar T·E" (como Netflix): el núcleo decide qué episodio toca.
  const upNext = data.episodes.find((e) => e.id === data.up_next_episode_id);

  return (
    <div className="relative min-h-full">
      <div className="absolute inset-0 h-[50vh] overflow-hidden" style={{ ["--trailer-h" as string]: "50vh" }}>
        {backdrop && <img src={backdrop} alt="" className="h-full w-full object-cover" />}
        {backdrop && data.trailer_key && <BackdropTrailer youtubeKey={data.trailer_key} />}
        {poster && <img src={poster} alt="" className="h-full w-full scale-110 object-cover opacity-40 blur-2xl" />}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/70 to-transparent ios:from-black ios:via-black/60" />
      </div>

      <div className={`relative flex flex-col gap-5 px-4 pb-16 sm:px-10 ${backdrop ? "pt-[26vh]" : "pt-10"}`}>
        <FloatingBackButton onClick={goBack} />
        <button type="button" onClick={goBack} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200 ios:hidden ios:desk:block">
          ← Volver <span className="pointer-coarse:hidden">(Esc)</span>
        </button>
        {poster && <img src={poster} alt="" className="w-36 rounded-lg shadow-2xl ring-1 ring-white/10 sm:w-44" />}
        <h1 className="max-w-2xl text-4xl font-bold sm:text-5xl">{data.clean_title}</h1>
        <div className="flex flex-wrap items-center gap-3 text-sm text-neutral-300">
          {data.age_rating && <span className="rounded border border-neutral-500 px-1.5 py-0.5 text-xs font-semibold">{data.age_rating}</span>}
          {data.year && <span>{data.year}</span>}
          {data.rating != null && <span>★ {data.rating.toFixed(1)}</span>}
        </div>
        {data.overview && <p className="max-w-2xl text-neutral-300">{data.overview}</p>}

        <div className="flex flex-col gap-3 pt-2 sm:flex-row sm:items-start">
          {upNext && (
            <button
              type="button"
              onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(upNext.id) }, search: { seriesId } })}
              className={PRIMARY_ACTION}
            >
              ▶ {data.up_next_started ? "Continuar" : "Ver"} T{upNext.season} · E{upNext.number}
            </button>
          )}
          <div className="grid auto-cols-fr grid-flow-col gap-1 sm:flex sm:gap-3">
            <button
              type="button"
              onClick={() => toggleFavorite("series", data.id, !isFavorite("series", data.id))}
              aria-pressed={isFavorite("series", data.id)}
              className={DETAIL_ACTION}
            >
              <HeartIcon filled={isFavorite("series", data.id)} className={`h-6 w-6 sm:h-5 sm:w-5 ${isFavorite("series", data.id) ? "text-red-500" : ""}`} />
              {isFavorite("series", data.id) ? "En mi lista" : "Mi lista"}
            </button>
            {/* Junto a «Mi lista» (como «Marcar vista» en la ficha de película): al lado de las
                temporadas les quitaba sitio y salían cortadas en el iPhone. */}
            {activeSeason !== null && (
              <button
                type="button"
                onClick={() => seasonWatchedMutation.mutate({ season: activeSeason, watched: !seasonWatched })}
                aria-pressed={seasonWatched}
                aria-label={seasonWatched ? "Marcar temporada como no vista" : "Marcar temporada como vista"}
                className={DETAIL_ACTION}
              >
                <span className="text-xl leading-none sm:text-base">{seasonWatched ? "✓" : "○"}</span>
                {seasonWatched ? `T${activeSeason} vista` : `Marcar T${activeSeason}`}
              </button>
            )}
            <RatingControl kind="series" id={data.id} title={data.clean_title} />
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

        {seasons.length === 0 ? (
          <p className="pt-6 text-sm text-neutral-500">
            Todavía no hay episodios descargados de esta serie. Se piden al proveedor la primera vez que se abre la ficha.
          </p>
        ) : (
          <div className="pt-6">
            <div className="mb-4">
              <div className="flex gap-2 overflow-x-auto" role="tablist" aria-label="Temporadas">
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
            </div>

            <ul className="flex flex-col gap-3">
              {episodesOfSeason.map((episode, index) => (
                <li key={episode.id}>
                  <button
                    type="button"
                    // Con mando, el foco llega al primer episodio de la temporada.
                    autoFocus={index === 0}
                    onClick={() =>
                      navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(episode.id) }, search: { seriesId } })
                    }
                    className="flex w-full gap-4 rounded-lg p-2 text-left outline-none hover:bg-neutral-900 focus-visible:bg-neutral-900"
                  >
                    <div className="flex h-20 w-32 shrink-0 items-center justify-center overflow-hidden rounded bg-neutral-800 text-xs text-neutral-500">
                      {imagePath(base, episode.still_path, "w342") ? (
                        <img
                          src={imagePath(base, episode.still_path, "w342")!}
                          alt=""
                          className="h-full w-full object-cover"
                          // Miniatura del proveedor caída: mejor el hueco gris que el icono roto.
                          onError={(e) => (e.currentTarget.style.visibility = "hidden")}
                        />
                      ) : (
                        episode.number
                      )}
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="flex items-center gap-2 truncate font-medium">
                        <span
                          role="button"
                          tabIndex={0}
                          aria-pressed={episode.watched}
                          aria-label={episode.watched ? "Marcar como no visto" : "Marcar como visto"}
                          title={episode.watched ? "Marcar como no visto" : "Marcar como visto"}
                          onClick={(e) => {
                            e.stopPropagation();
                            episodeWatchedMutation.mutate({ episodeId: episode.id, watched: !episode.watched });
                          }}
                          onKeyDown={(e) => {
                            if (e.key !== "Enter" && e.key !== " ") return;
                            e.stopPropagation();
                            e.preventDefault();
                            episodeWatchedMutation.mutate({ episodeId: episode.id, watched: !episode.watched });
                          }}
                          className={`shrink-0 rounded-full text-lg outline-none ${
                            episode.watched ? "text-emerald-500 hover:text-emerald-400" : "text-neutral-600 hover:text-neutral-400"
                          }`}
                        >
                          ✓
                        </span>
                        <span className="truncate">
                          {episode.number}. {episode.title || `Episodio ${episode.number}`}
                        </span>
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

      {data.similar.length > 0 && (
        <div className="relative pb-10">
          <Row title="Más como esto">
            {data.similar.map((title) => (
              <PosterCard
                key={title.id}
                title={title.clean_title}
                posterPath={title.poster_path}
                subtitle={title.year ? String(title.year) : undefined}
                onOpen={() => navigate({ to: "/series/$seriesId", params: { seriesId: String(title.id) } })}
              />
            ))}
          </Row>
        </div>
      )}
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
