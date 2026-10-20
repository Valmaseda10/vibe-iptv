import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { apiDelete, apiGetOrNull, apiPost, errorMessage, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type {
  ChannelsPage,
  EpgNowNextResponse,
  EpisodeSummary,
  MovieDetailResponse,
  NativePlayItem,
  NativePlayerEvent,
  NativePlayRequest,
  PlaybackProgressResponse,
  SeriesDetailResponse,
  StreamFailure,
} from "../../lib/api-types";
import { findNextEpisode } from "./episodes";

type Kind = "channel" | "movie" | "episode";

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
export function NativePlayerScreen() {
  const { kind, id } = useParams({ from: "/player/$kind/$id" }) as { kind: Kind; id: string };
  const search = useSearch({ from: "/player/$kind/$id" });
  const navigate = useNavigate();
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
  const channelsQuery = useQuery({
    queryKey: ["player-channels", profileId, search.categoryId],
    queryFn: () =>
      apiGetOrNull<ChannelsPage>(
        `/api/live/channels?profile_id=${profileId}&limit=200${search.categoryId ? `&category_id=${search.categoryId}` : ""}`,
      ),
    enabled: kind === "channel" && profileId !== null,
  });
  const channelIds = useMemo(() => (channelsQuery.data?.items ?? []).map((c) => c.id).join(","), [channelsQuery.data]);
  const epgNowQuery = useQuery({
    queryKey: ["player-epg-now-list", channelIds],
    queryFn: () => apiGetOrNull<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${channelIds}`),
    enabled: kind === "channel" && !isCatchup && channelIds.length > 0,
    staleTime: 60_000,
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
  const currentChannel = channelsQuery.data?.items.find((c) => c.id === itemId);

  const title =
    kind === "movie" ? movieQuery.data?.clean_title : kind === "episode" ? seriesQuery.data?.clean_title : currentChannel?.display_name;
  const subtitle =
    kind === "episode" && currentEpisode ? episodeLabel(currentEpisode) : kind === "channel" ? (nowByChannel.get(itemId) ?? null) : null;

  const channelPath = useCallback((channelId: number) => `/stream/live-buffer/channel/${channelId}?profile_id=${profileId}`, [profileId]);
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
  const currentIndex = items.findIndex((item) => item.id === itemId);
  const next: NativePlayItem | null =
    kind === "episode" && nextEpisode && base
      ? { id: nextEpisode.id, title: seriesQuery.data?.clean_title ?? "", subtitle: episodeLabel(nextEpisode), number: null, url: `${base}${episodePath(nextEpisode.id)}` }
      : null;

  const ready = base != null && profileId !== null && startSeconds !== null && (title != null || titleWaitOver);
  const playedPathRef = useRef<string | null>(null);
  const lastRequestRef = useRef<NativePlayRequest | null>(null);

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
      update_only: false,
    };
  }, [base, startSeconds, profileId, streamPath, title, subtitle, kind, isCatchup, liveBuffer, items, currentIndex, next]);

  const play = useCallback(() => {
    const request = buildRequest();
    if (!request) return;
    lastRequestRef.current = request;
    setError(null);
    invoke("native_player_play", { request }).catch((err: unknown) => setError(errorMessage(err)));
  }, [buildRequest]);

  useEffect(() => {
    if (!ready || playedPathRef.current === streamPath) return;
    playedPathRef.current = streamPath;
    play();
  }, [ready, streamPath, play]);

  // La lista o el siguiente episodio llegan (o cambian) con el reproductor ya abierto: se
  // le pasan sin recargar el vídeo.
  const itemsKey = `${items.map((i) => `${i.id}:${i.subtitle ?? ""}`).join("|")}#${currentIndex}#${next?.id ?? ""}`;
  useEffect(() => {
    if (!lastRequestRef.current || playedPathRef.current !== streamPath) return;
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
  const latestRef = useRef({ kind, itemId, profileId, channels: channelsQuery.data?.items, search, goBack, isCatchup, channelPath, episodePath });
  useEffect(() => {
    latestRef.current = { kind, itemId, profileId, channels: channelsQuery.data?.items, search, goBack, isCatchup, channelPath, episodePath };
  });

  useEffect(() => {
    const saveProgress = (position: number, duration: number | null) => {
      const { kind, itemId, profileId } = latestRef.current;
      if (kind === "channel" || profileId === null || position <= 0) return;
      apiPost("/api/playback/progress", {
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
          saveProgress(payload.position_s, payload.duration_s);
          break;
        case "closed": {
          const position = payload.ended && payload.duration_s ? payload.duration_s : payload.position_s;
          saveProgress(position, payload.duration_s);
          playedPathRef.current = null;
          lastRequestRef.current = null;
          // Suelta ya la conexión con el proveedor: la grabación del directo no tiene que
          // esperar a quedarse sin lectores.
          if (latest.kind === "channel") apiDelete("/api/live-buffer").catch(() => {});
          latest.goBack();
          break;
        }
        case "switched": {
          // El reproductor ya está reproduciendo el nuevo canal o episodio: la interfaz solo
          // se pone al día (ruta, recientes, progreso) sin pedirle que vuelva a empezar.
          if (latest.kind === "channel") {
            playedPathRef.current = latest.channelPath(payload.id);
            navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(payload.id) }, search: { categoryId: latest.search.categoryId } });
          } else if (latest.kind === "episode") {
            playedPathRef.current = latest.episodePath(payload.id);
            navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(payload.id) }, search: { seriesId: latest.search.seriesId } });
          }
          break;
        }
        case "tracks":
          saveTrackPreference(latest.profileId, { audio: payload.audio, subtitle: payload.subtitle });
          break;
        case "zap": {
          const list = latest.channels ?? [];
          const index = list.findIndex((c) => c.id === latest.itemId);
          const target = index === -1 ? undefined : list[index + payload.direction];
          if (target) {
            navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(target.id) }, search: { categoryId: latest.search.categoryId } });
          }
          break;
        }
        case "error": {
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
  }, [navigate]);

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
