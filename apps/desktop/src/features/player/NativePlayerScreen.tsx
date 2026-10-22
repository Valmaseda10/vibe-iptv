import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { failoverNotice, nextUntriedSource, sourceLabel } from "./movieSources";
import { readSubtitleStyle, saveSubtitleStyle } from "../../lib/subtitleStyle";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { apiGetOrNull, apiPost, errorMessage, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type {
  CategorySummary,
  ChannelSummary,
  ChannelsPage,
  EpgNowNextResponse,
  EpisodeSummary,
  MovieDetailResponse,
  NativeListOption,
  NativePlayItem,
  NativePlayerEvent,
  NativePlayRequest,
  PlaybackProgressResponse,
  SeriesDetailResponse,
  StreamFailure,
} from "../../lib/api-types";
import { findNextEpisode } from "./episodes";
import { feedLabels } from "../../lib/channels";

type Kind = "channel" | "movie" | "episode";

/** Lista de canales del reproductor: una categoría, los favoritos o todas. */
type ListScope = { categoryId?: string; favorites?: boolean };
const FAVORITES_SCOPE = "fav";

const RESUME_MIN_SECONDS = 5;
/** Si el título no llega en este tiempo (núcleo lento, ficha sin datos), se reproduce igual. */
const TITLE_WAIT_MS = 2_000;

/** Pistas de audio y subtítulos que eligió cada perfil, para repetirlas (F15). */
type TrackPreference = { audio: string | null; subtitle: string | null };

function readTrackPreference(profileId: number | null): TrackPreference {
  if (profileId === null) return { audio: null, subtitle: null };
  try {
    const raw = localStorage.getItem(`vibe.tracks.${profileId}`);
    return raw ? (JSON.parse(raw) as TrackPreference) : { audio: null, subtitle: null };
  } catch {
    return { audio: null, subtitle: null };
  }
}

function saveTrackPreference(profileId: number | null, preference: TrackPreference) {
  if (profileId === null) return;
  try {
    localStorage.setItem(`vibe.tracks.${profileId}`, JSON.stringify(preference));
  } catch {
    // Sin almacenamiento (modo privado): no se recuerda, no pasa nada.
  }
}

function episodeLabel(episode: EpisodeSummary): string {
  return `T${episode.season} · ${episode.number}. ${episode.title ?? `Episodio ${episode.number}`}`;
}

/** Reproducción en iPhone y iPad (F15). La hace VLC en una pantalla nativa que tapa
 * esta (ver `src-tauri/src/native_player.rs`); aquí se decide qué reproducir, se le pasan
 * la lista de canales o episodios (el zapping lo hace el propio reproductor, al instante),
 * el siguiente episodio y las pistas preferidas, y se guarda el progreso que cuenta.
 *
 * El VOD va con `passthrough=true` (el núcleo no tiene ffmpeg en iOS: VLC abre cualquier
 * contenedor); el directo, desde la grabación del núcleo para poder pausarlo. */
/** Datos que cambian al ver algo: fichas («Continuar», «Quedan»), Inicio («Continuar
 * viendo»), el progreso y los canales recientes. */
const WATCH_DEPENDENT_QUERY_KEYS = ["movie", "series", "home", "browse", "progress", "live-recent", "player-series", "player-movie"];

export function NativePlayerScreen() {
  const { kind, id } = useParams({ from: "/player/$kind/$id" }) as { kind: Kind; id: string };
  const search = useSearch({ from: "/player/$kind/$id" });
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: base } = useApiBase();
  const { profileId } = useActiveProfile();
  const itemId = Number(id);
  const isCatchup = kind === "channel" && search.catchupStart !== undefined;

  const [startSeconds, setStartSeconds] = useState<number | null>(kind === "channel" ? 0 : null);
  const [titleWaitOver, setTitleWaitOver] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const movieQuery = useQuery({
    queryKey: ["player-movie", itemId, profileId],
    queryFn: () => apiGetOrNull<MovieDetailResponse>(`/api/movies/${itemId}?profile_id=${profileId}`),
    enabled: kind === "movie" && profileId !== null,
  });
  const seriesQuery = useQuery({
    queryKey: ["player-series", search.seriesId, profileId],
    queryFn: () => apiGetOrNull<SeriesDetailResponse>(`/api/series/${search.seriesId}?profile_id=${profileId}`),
    enabled: kind === "episode" && Boolean(search.seriesId) && profileId !== null,
  });
  // Los canales de alrededor del que se ve (nombre, lista y zapping), no la primera página:
  // en listas de miles el canal podía no estar en ella. La ventana da la vuelta por los
  // extremos y se recentra en cada cambio de canal; mientras llega la nueva, sigue la vieja.
  // Lista elegida en el selector de categoría del reproductor; mientras no se toca, la
  // de la ruta (con la que se abrió).
  const [listScope, setListScope] = useState<ListScope | null>(null);
  const scope: ListScope = listScope ?? { categoryId: search.categoryId, favorites: search.favorites };
  // Otra fuente del canal puede no estar en la lista (otra categoría): la lista y el
  // zapping siguen anclados al canal de la lista, como en el PC.
  const [feedAnchor, setFeedAnchor] = useState<{ feed: number; anchor: number } | null>(null);
  const listChannelId = kind === "channel" && feedAnchor?.feed === itemId ? feedAnchor.anchor : itemId;
  const channelsQuery = useQuery({
    queryKey: ["player-channels", profileId, scope.categoryId, scope.favorites, listChannelId],
    queryFn: () =>
      apiGetOrNull<ChannelsPage>(
        `/api/live/channels?profile_id=${profileId}&limit=100&around=${listChannelId}${scope.favorites ? "&favorites=1" : scope.categoryId ? `&category_id=${scope.categoryId}` : ""}`,
      ),
    placeholderData: keepPreviousData,
    enabled: kind === "channel" && profileId !== null,
  });
  const categoriesQuery = useQuery({
    queryKey: ["live-categories", profileId],
    queryFn: () => apiGetOrNull<CategorySummary[]>(`/api/live/categories?profile_id=${profileId}`),
    enabled: kind === "channel" && profileId !== null,
    staleTime: 5 * 60_000,
  });
  // Las fuentes del canal (sus otras calidades), para el menú «Más» → Fuente.
  const variantsQuery = useQuery({
    queryKey: ["player-channel-variants", listChannelId, profileId],
    queryFn: () => apiGetOrNull<ChannelSummary[]>(`/api/live/channels/${listChannelId}/variants?profile_id=${profileId}`),
    enabled: kind === "channel" && profileId !== null,
    staleTime: 5 * 60_000,
  });
  const channelFeeds = useMemo(() => (kind === "channel" ? (variantsQuery.data ?? []) : []), [kind, variantsQuery.data]);
  const channelIds = useMemo(() => (channelsQuery.data?.items ?? []).map((c) => c.id).join(","), [channelsQuery.data]);
  const epgNowQuery = useQuery({
    queryKey: ["player-epg-now-list", channelIds],
    queryFn: () => apiGetOrNull<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${channelIds}`),
    enabled: kind === "channel" && !isCatchup && channelIds.length > 0,
    staleTime: 60_000,
    // La ventana de canales se recentra al cambiar de canal: lo que emiten los de antes
    // sigue a la vista hasta que llega lo nuevo.
    placeholderData: keepPreviousData,
  });

  const nowByChannel = useMemo(() => {
    const map = new Map<number, string>();
    for (const entry of epgNowQuery.data ?? []) if (entry.current) map.set(entry.channel_id, entry.current.title);
    return map;
  }, [epgNowQuery.data]);

  const episodes = useMemo(
    () => [...(seriesQuery.data?.episodes ?? [])].sort((a, b) => a.season - b.season || a.number - b.number),
    [seriesQuery.data],
  );
  const currentEpisode = episodes.find((e) => e.id === itemId);
  const nextEpisode = useMemo(() => (currentEpisode ? findNextEpisode(episodes, currentEpisode) : undefined), [episodes, currentEpisode]);
  const currentChannel = channelsQuery.data?.items.find((c) => c.id === listChannelId) ?? channelFeeds.find((c) => c.id === itemId);

  const title =
    kind === "movie" ? movieQuery.data?.clean_title : kind === "episode" ? seriesQuery.data?.clean_title : currentChannel?.display_name;
  const subtitle =
    kind === "episode" && currentEpisode ? episodeLabel(currentEpisode) : kind === "channel" ? (nowByChannel.get(itemId) ?? null) : null;

  // Una sesión por reproductor abierto: al cerrarlo, el núcleo deja de servir sus URLs y
  // una reconexión tardía de VLC ya no vuelve a abrir el directo con el proveedor.
  const [session] = useState(() => `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`);
  const channelPath = useCallback(
    (channelId: number) => `/stream/live-buffer/channel/${channelId}?profile_id=${profileId}&session=${session}`,
    [profileId, session],
  );
  const episodePath = useCallback((episodeId: number) => `/stream/episode/${episodeId}?profile_id=${profileId}&passthrough=true`, [profileId]);

  // Continuar viendo: el VOD arranca donde se dejó.
  useEffect(() => {
    if (kind === "channel" || profileId === null) return;
    let cancelled = false;
    setStartSeconds(null);
    apiGetOrNull<PlaybackProgressResponse>(`/api/playback/progress?profile_id=${profileId}&item_kind=${kind}&item_id=${itemId}`)
      .catch(() => null)
      .then((progress) => {
        if (cancelled) return;
        setStartSeconds(progress && !progress.finished && progress.position_s > RESUME_MIN_SECONDS ? progress.position_s : 0);
      });
    return () => {
      cancelled = true;
    };
  }, [kind, itemId, profileId]);

  useEffect(() => {
    setTitleWaitOver(false);
    const timer = window.setTimeout(() => setTitleWaitOver(true), TITLE_WAIT_MS);
    return () => window.clearTimeout(timer);
  }, [kind, itemId]);

  // El directo sale de la grabación del núcleo (se puede pausar y rebobinar, F15); el
  // catch-up y el VOD, tal cual del proveedor.
  const liveBuffer = kind === "channel" && !isCatchup;
  const streamPath = isCatchup
    ? `/stream/catchup/${itemId}?profile_id=${profileId}&start=${search.catchupStart}${search.catchupDuration ? `&duration_min=${search.catchupDuration}` : ""}&passthrough=true`
    : liveBuffer
      ? channelPath(itemId)
      : kind === "episode"
        ? episodePath(itemId)
        : `/stream/${kind}/${itemId}?profile_id=${profileId}${search.source ? `&source=${search.source}` : ""}&passthrough=true`;

  // Lo que el reproductor puede abrir por su cuenta: los canales de la categoría (con lo
  // que emiten) o los episodios de la serie, y el siguiente episodio.
  const items: NativePlayItem[] = useMemo(() => {
    if (!base) return [];
    if (liveBuffer) {
      return (channelsQuery.data?.items ?? []).map((c) => ({
        id: c.id,
        title: c.display_name,
        subtitle: nowByChannel.get(c.id) ?? null,
        number: c.number ?? null,
        url: `${base}${channelPath(c.id)}`,
      }));
    }
    if (kind === "episode") {
      return episodes.map((e) => ({ id: e.id, title: seriesQuery.data?.clean_title ?? "", subtitle: episodeLabel(e), number: null, url: `${base}${episodePath(e.id)}` }));
    }
    return [];
  }, [base, liveBuffer, kind, channelsQuery.data, nowByChannel, episodes, seriesQuery.data, channelPath, episodePath]);
  const currentIndex = items.findIndex((item) => item.id === listChannelId);
  const listOptions: NativeListOption[] = useMemo(
    () =>
      liveBuffer
        ? [
            { id: "", name: "Todas las categorías" },
            { id: FAVORITES_SCOPE, name: "★ Favoritos" },
            ...(categoriesQuery.data ?? []).map((c) => ({ id: String(c.id), name: c.name })),
          ]
        : [],
    [liveBuffer, categoriesQuery.data],
  );
  const scopeId = scope.favorites ? FAVORITES_SCOPE : (scope.categoryId ?? "");
  const feedIndex = channelFeeds.findIndex((c) => c.id === itemId);
  const next: NativePlayItem | null =
    kind === "episode" && nextEpisode && base
      ? { id: nextEpisode.id, title: seriesQuery.data?.clean_title ?? "", subtitle: episodeLabel(nextEpisode), number: null, url: `${base}${episodePath(nextEpisode.id)}` }
      : null;

  // Fuentes de la película (F15): se cambia a mano desde el reproductor y, si una no carga,
  // se prueba sola la siguiente (cada una una vez).
  const movieSources = useMemo(() => (kind === "movie" ? (movieQuery.data?.sources ?? []) : []), [kind, movieQuery.data]);
  const activeSourceId = search.source ?? movieSources[0]?.id ?? null;
  const activeSourceIndex = Math.max(0, movieSources.findIndex((source) => source.id === activeSourceId));
  const triedSourcesRef = useRef<Set<number>>(new Set());
  const lastPositionRef = useRef(0);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    triedSourcesRef.current = new Set();
    lastPositionRef.current = 0;
  }, [kind, itemId]);
  const switchSource = useCallback(
    (index: number, auto: boolean) => {
      const target = movieSources[index];
      if (!target || target.id === activeSourceId) return;
      if (auto) setNotice(failoverNotice(activeSourceIndex, index));
      // Sigue en el mismo minuto, con la otra fuente.
      setStartSeconds(Math.floor(lastPositionRef.current));
      navigate({ to: "/player/$kind/$id", params: { kind: "movie", id }, search: { ...search, source: target.id }, replace: true });
    },
    [movieSources, activeSourceId, activeSourceIndex, navigate, id, search],
  );

  const ready = base != null && profileId !== null && startSeconds !== null && (title != null || titleWaitOver);
  const playedPathRef = useRef<string | null>(null);
  const lastRequestRef = useRef<NativePlayRequest | null>(null);
  // El reproductor ya se cerró y esta pantalla se está yendo: no se vuelve a pedir nada.
  // Sin esto, un repintado antes de salir (al refrescar la ficha) volvía a abrir el vídeo,
  // a veces de forma invisible, con el directo abierto con el proveedor (visto en el
  // simulador). Se rearma solo si cambia lo que hay que ver.
  const closedRef = useRef(false);
  useEffect(() => {
    closedRef.current = false;
  }, [kind, itemId]);

  const buildRequest = useCallback((): NativePlayRequest | null => {
    if (!base || startSeconds === null) return null;
    const tracks = readTrackPreference(profileId);
    return {
      url: `${base}${streamPath}`,
      title: title ?? "",
      subtitle,
      start_seconds: startSeconds,
      live: kind === "channel" && !isCatchup,
      live_buffer: liveBuffer,
      message: null,
      kind,
      items,
      current_index: currentIndex >= 0 ? currentIndex : null,
      next,
      preferred_audio: tracks.audio,
      preferred_subtitle: tracks.subtitle,
      subtitle_style: readSubtitleStyle(profileId),
      sources:
        movieSources.length > 1 ? movieSources.map(sourceLabel) : liveBuffer && channelFeeds.length > 1 ? feedLabels(channelFeeds) : [],
      current_source: movieSources.length > 1 ? activeSourceIndex : liveBuffer && channelFeeds.length > 1 && feedIndex >= 0 ? feedIndex : null,
      notice,
      update_only: false,
      categories: listOptions,
      current_category: liveBuffer ? scopeId : null,
    };
  }, [base, startSeconds, profileId, streamPath, title, subtitle, kind, isCatchup, liveBuffer, items, currentIndex, next, movieSources, activeSourceIndex, notice, channelFeeds, feedIndex, listOptions, scopeId]);

  const play = useCallback(() => {
    const request = buildRequest();
    if (!request) return;
    lastRequestRef.current = request;
    setError(null);
    setNotice(null);
    invoke("native_player_play", { request }).catch((err: unknown) => setError(errorMessage(err)));
  }, [buildRequest]);

  useEffect(() => {
    if (!ready || closedRef.current || playedPathRef.current === streamPath) return;
    playedPathRef.current = streamPath;
    play();
  }, [ready, streamPath, play]);

  // La lista o el siguiente episodio llegan (o cambian) con el reproductor ya abierto: se
  // le pasan sin recargar el vídeo.
  const itemsKey = `${items.map((i) => `${i.id}:${i.subtitle ?? ""}`).join("|")}#${currentIndex}#${next?.id ?? ""}#${scopeId}#${listOptions.length}#${channelFeeds.map((c) => c.id).join(",")}`;
  useEffect(() => {
    if (closedRef.current || !lastRequestRef.current || playedPathRef.current !== streamPath) return;
    const request = buildRequest();
    if (!request) return;
    invoke("native_player_play", { request: { ...request, update_only: true } }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemsKey]);

  const goBack = useCallback(() => {
    if (kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: id } });
    else if (kind === "episode" && search.seriesId) navigate({ to: "/series/$seriesId", params: { seriesId: search.seriesId } });
    else navigate({ to: isCatchup ? "/guide" : "/live" });
  }, [kind, id, search.seriesId, isCatchup, navigate]);

  // Los eventos llegan a un solo oyente; lo que cambia de un render a otro se lee de aquí.
  // Otra calidad del mismo canal, en el sitio: la lista y el zapping no cambian.
  const switchChannelFeed = useCallback(
    (index: number) => {
      const target = channelFeeds[index];
      if (!target || target.id === itemId) return;
      setFeedAnchor({ feed: target.id, anchor: listChannelId });
      navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(target.id) }, search, replace: true });
    },
    [channelFeeds, itemId, listChannelId, navigate, search],
  );

  const latestRef = useRef({ kind, itemId, profileId, channels: channelsQuery.data?.items, search, scope, listChannelId, goBack, isCatchup, channelPath, episodePath, movieSources, activeSourceId, switchSource, switchChannelFeed });
  useEffect(() => {
    latestRef.current = { kind, itemId, profileId, channels: channelsQuery.data?.items, search, scope, listChannelId, goBack, isCatchup, channelPath, episodePath, movieSources, activeSourceId, switchSource, switchChannelFeed };
  });

  useEffect(() => {
    const saveProgress = (position: number, duration: number | null, eventId: number | null) => {
      const { kind, profileId } = latestRef.current;
      // El reproductor dice a qué episodio se refiere: tras un cambio dentro de él, la
      // interfaz puede ir un instante por detrás.
      const itemId = eventId ?? latestRef.current.itemId;
      if (kind === "channel" || profileId === null || position <= 0) return Promise.resolve();
      return apiPost("/api/playback/progress", {
        profile_id: profileId,
        item_kind: kind,
        item_id: itemId,
        position_s: Math.floor(position),
        duration_s: duration ? Math.floor(duration) : null,
      }).catch(() => {});
    };

    const unlisten = listen<NativePlayerEvent>("native-player", ({ payload }) => {
      const latest = latestRef.current;
      switch (payload.type) {
        case "progress":
          if (payload.position_s > 0) lastPositionRef.current = payload.position_s;
          saveProgress(payload.position_s, payload.duration_s, payload.id);
          break;
        case "source":
          if (latest.kind === "channel") latest.switchChannelFeed(payload.index);
          else latest.switchSource(payload.index, false);
          break;
        case "category":
          setListScope(payload.id === FAVORITES_SCOPE ? { favorites: true } : payload.id ? { categoryId: payload.id } : {});
          break;
        case "closed": {
          const position = payload.ended && payload.duration_s ? payload.duration_s : payload.position_s;
          const saved = saveProgress(position, payload.duration_s, payload.id);
          closedRef.current = true;
          lastRequestRef.current = null;
          // La grabación del directo la suelta el propio reproductor (VibePlayer.swift) en
          // cuanto VLC ha parado: pedirlo desde aquí, antes, hacía que VLC reconectara.
          latest.goBack();
          // Lo visto cambia la ficha ("Continuar", "Quedan N min"), Inicio y los recientes:
          // sin esto, la ficha seguía con los datos de antes de ver nada.
          // Solo lo que depende de lo visto: refrescar todo hacía que la app tardara en
          // volver a responder al salir de una película (visto en un iPhone).
          saved.finally(() => {
            for (const key of WATCH_DEPENDENT_QUERY_KEYS) queryClient.invalidateQueries({ queryKey: [key] });
          });
          break;
        }
        case "switched": {
          // El reproductor ya está reproduciendo el nuevo canal o episodio: la interfaz solo
          // se pone al día (ruta, recientes, progreso) sin pedirle que vuelva a empezar.
          if (latest.kind === "channel") {
            playedPathRef.current = latest.channelPath(payload.id);
            // La lista elegida en el selector pasa a ser la del zapping.
            navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(payload.id) }, search: { categoryId: latest.scope.categoryId, favorites: latest.scope.favorites } });
            setListScope(null);
          } else if (latest.kind === "episode") {
            playedPathRef.current = latest.episodePath(payload.id);
            navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(payload.id) }, search: { seriesId: latest.search.seriesId } });
          }
          break;
        }
        case "tracks":
          saveTrackPreference(latest.profileId, { audio: payload.audio, subtitle: payload.subtitle });
          break;
        case "subtitle_style":
          saveSubtitleStyle(latest.profileId, payload.style);
          break;
        case "zap": {
          const list = latest.channels ?? [];
          const index = list.findIndex((c) => c.id === latest.listChannelId);
          // Tras el último canal viene el primero (y al revés), como en TiviMate. Si el que
          // se ve no está en la lista elegida, se empieza por el primero.
          const target = list.length === 0 ? undefined : index === -1 ? list[0] : list.length < 2 ? undefined : list[(((index + payload.direction) % list.length) + list.length) % list.length];
          if (target) {
            navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(target.id) }, search: { categoryId: latest.scope.categoryId, favorites: latest.scope.favorites } });
            setListScope(null);
          }
          break;
        }
        case "error": {
          // Película con más fuentes: se prueba la siguiente sin probar, desde el mismo punto.
          if (latest.kind === "movie" && latest.movieSources.length > 1) {
            if (latest.activeSourceId != null) triedSourcesRef.current.add(latest.activeSourceId);
            const nextIndex = nextUntriedSource(latest.movieSources, triedSourcesRef.current);
            if (nextIndex >= 0) {
              latest.switchSource(nextIndex, true);
              break;
            }
          }
          // VLC solo sabe que no pudo; el núcleo sabe por qué (bloqueo por país, cuenta en
          // uso, caducada...). Se pide y se enseña en el propio reproductor.
          setError(payload.message);
          const failureKind = latest.isCatchup ? "catchup" : latest.kind;
          apiGetOrNull<StreamFailure>(`/api/playback/last-error/${failureKind}/${latest.itemId}`)
            .then((failure) => {
              const request = lastRequestRef.current;
              if (!failure?.message || !request) return;
              setError(failure.message);
              invoke("native_player_play", { request: { ...request, message: failure.message } }).catch(() => {});
            })
            .catch(() => {});
          break;
        }
      }
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [navigate, queryClient]);

  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-4 bg-black px-4 text-center text-neutral-300">
      <p className="text-lg font-medium text-white">{title ?? "Cargando…"}</p>
      {subtitle && <p className="text-sm text-neutral-400">{subtitle}</p>}
      {error && <p className="max-w-md text-sm text-red-400">{error}</p>}
      <div className="flex gap-3">
        <button
          type="button"
          onClick={play}
          disabled={!ready}
          className="rounded-md bg-white px-4 py-2 text-sm font-semibold text-black outline-none focus-visible:ring-2 focus-visible:ring-red-500 disabled:opacity-40"
        >
          Reproducir
        </button>
        <button
          type="button"
          onClick={goBack}
          className="rounded-md bg-neutral-800 px-4 py-2 text-sm font-semibold text-white outline-none focus-visible:ring-2 focus-visible:ring-red-500"
        >
          Volver
        </button>
      </div>
    </div>
  );
}
