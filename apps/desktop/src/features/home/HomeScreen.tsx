import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import { apiDelete, apiGet, imagePath, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { FeaturedItem, HomeResponse, ProviderSummary } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { PosterCard, PosterCardSkeleton } from "../../components/PosterCard";
import { ProviderStatusPill } from "./ProviderStatusPill";

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

  return (
    <div className="relative flex flex-col gap-8 pb-8 sm:gap-10">
      {/* Arriba a la derecha, sobre el banner: ¿responde la lista ahora mismo? */}
      <div className="absolute right-4 top-3 z-20 sm:right-10 sm:top-5">
        <ProviderStatusPill />
      </div>
      {isPending ? <HeroBannerSkeleton /> : data?.featured ? <HeroBanner featured={data.featured} /> : null}

      {/* Sin banner (sin clave de TMDB no hay fondos), el título necesita su propio margen. */}
      <h1
        className={`px-4 text-2xl font-semibold sm:px-10 sm:text-3xl ${!isPending && !data?.featured ? "pt-6 sm:pt-8" : "hidden sm:block"}`}
      >
        Inicio
      </h1>

      {isError && <p className="px-4 text-red-400 sm:px-10">No se pudo cargar el inicio. ¿Hay algún proveedor sincronizado?</p>}

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
              progressPercent={progressPercent(item.position_s, item.duration_s)}
              onOpen={() => openContinueWatching(item.item_kind, item.item_id)}
              onRemove={() => removeMutation.mutate({ item_kind: item.item_kind, item_id: item.item_id })}
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

      {!isPending && data && data.recommended.length > 0 && (
        <Row title="Recomendado para ti">
          {data.recommended.map((movie) => (
            <PosterCard
              key={`rec-${movie.id}`}
              title={movie.clean_title}
              posterPath={movie.poster_path}
              subtitle={`Porque te gustó ${movie.because_of}`}
              onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(movie.id) } })}
            />
          ))}
        </Row>
      )}

      {!isPending && data && data.recommended_series.length > 0 && (
        <Row title="Series recomendadas para ti">
          {data.recommended_series.map((series) => (
            <PosterCard
              key={`rec-series-${series.id}`}
              title={series.clean_title}
              posterPath={series.poster_path}
              subtitle={`Porque te gustó ${series.because_of}`}
              onOpen={() => navigate({ to: "/series/$seriesId", params: { seriesId: String(series.id) } })}
            />
          ))}
        </Row>
      )}

      {/* "Tendencias ahora" (F "premium"): lista externa de TMDB, no depende de nada
          visto en esta app — a diferencia de "Recomendado para ti". Vacía sin clave de
          TMDB o si ningún título de la lista está todavía en el catálogo local. */}
      {!isPending && data && data.trending_movies.length > 0 && (
        <Row title="Tendencias ahora">
          {data.trending_movies.map((movie) => (
            <PosterCard
              key={`trend-movie-${movie.id}`}
              title={movie.clean_title}
              posterPath={movie.poster_path}
              subtitle={movie.year ? String(movie.year) : undefined}
              onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(movie.id) } })}
            />
          ))}
        </Row>
      )}

      {!isPending && data && data.trending_series.length > 0 && (
        <Row title="Series en tendencia">
          {data.trending_series.map((series) => (
            <PosterCard
              key={`trend-series-${series.id}`}
              title={series.clean_title}
              posterPath={series.poster_path}
              subtitle={series.year ? String(series.year) : undefined}
              onOpen={() => navigate({ to: "/series/$seriesId", params: { seriesId: String(series.id) } })}
            />
          ))}
        </Row>
      )}

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
              isNew={movie.is_new}
              onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(movie.id) } })}
            />
          ))}
          {data && data.recent_movies.length === 0 && <p className="text-neutral-500">Todavía no hay películas sincronizadas.</p>}
        </Row>
      )}

      {/* Los géneros con más películas en el catálogo de verdad, no una lista fija —
          vacío hasta que corra el emparejamiento con TMDB al menos una vez. */}
      {!isPending &&
        data?.genre_rows.map((row) => (
          <Row key={row.name} title={row.name}>
            {row.movies.map((movie) => (
              <PosterCard
                key={movie.id}
                title={movie.clean_title}
                posterPath={movie.poster_path}
                subtitle={movie.year ? String(movie.year) : undefined}
                isNew={movie.is_new}
                onOpen={() => navigate({ to: "/movies/$movieId", params: { movieId: String(movie.id) } })}
              />
            ))}
          </Row>
        ))}
    </div>
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

/** Cabecera grande de Inicio, con el fondo de una película destacada — como cualquier
 * plataforma real, en vez de empezar directo con filas de pósters pequeños. Se recorta
 * con un degradado hacia el fondo de la app para que las filas de debajo no choquen. */
function HeroBanner({ featured }: { featured: FeaturedItem }) {
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const backdrop = imagePath(base, featured.backdrop_path, "w1280");

  return (
    <div className="relative h-[56vh] min-h-[340px] w-full overflow-hidden sm:h-[60vh] sm:min-h-[360px]">
      {backdrop && <img src={backdrop} alt="" className="absolute inset-0 h-full w-full object-cover" />}
      <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/40 to-transparent" />
      <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/80 via-transparent to-transparent" />

      <div className="absolute bottom-0 left-0 flex max-w-xl flex-col gap-4 px-4 pb-8 sm:px-10 sm:pb-10">
        <h2 className="text-3xl font-bold drop-shadow-lg sm:text-5xl">{featured.clean_title}</h2>
        {featured.overview && <p className="line-clamp-3 text-sm text-neutral-200 drop-shadow sm:text-base">{featured.overview}</p>}
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(featured.id) } })}
            className="flex items-center gap-2 rounded-lg bg-white px-6 py-2.5 font-semibold text-black outline-none transition hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-red-600"
          >
            ▶ Reproducir
          </button>
          <button
            type="button"
            onClick={() => navigate({ to: "/movies/$movieId", params: { movieId: String(featured.id) } })}
            className="flex items-center gap-2 rounded-lg bg-neutral-700/80 px-6 py-2.5 font-semibold text-white outline-none transition hover:bg-neutral-600/80 focus-visible:ring-2 focus-visible:ring-red-600"
          >
            Más información
          </button>
        </div>
      </div>
    </div>
  );
}

function HeroBannerSkeleton() {
  return (
    <div className="relative flex h-[60vh] min-h-[360px] w-full animate-pulse items-end overflow-hidden bg-neutral-900">
      <div className="flex w-full max-w-xl flex-col gap-4 px-4 pb-8 sm:px-10 sm:pb-10">
        <div className="h-10 w-2/3 rounded bg-neutral-800" />
        <div className="h-4 w-full rounded bg-neutral-800" />
        <div className="h-4 w-5/6 rounded bg-neutral-800" />
        <div className="h-10 w-48 rounded bg-neutral-800" />
      </div>
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
