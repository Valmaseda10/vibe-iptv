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
  MovieDetailResponse,
  NativePlayerEvent,
  NativePlayRequest,
  PlaybackProgressResponse,
  SeriesDetailResponse,
} from "../../lib/api-types";
import { findNextEpisode } from "./episodes";

type Kind = "channel" | "movie" | "episode";

const RESUME_MIN_SECONDS = 5;
/** Si el título no llega en este tiempo (núcleo lento, ficha sin datos), se reproduce igual. */
const TITLE_WAIT_MS = 2_000;

/** Reproducción en iPhone y iPad (F15). La hace VLC en una pantalla nativa que tapa
 * esta (ver `src-tauri/src/native_player.rs`); aquí solo se decide qué reproducir, se
 * guarda el progreso que cuenta el reproductor y se responde al zapping y al cierre.
 *
 * El stream va con `passthrough=true`: el núcleo no tiene ffmpeg en iOS y VLC abre
 * cualquier contenedor y códec por sí mismo. */
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
  const channelEpgQuery = useQuery({
    queryKey: ["player-epg-now", itemId],
    queryFn: () => apiGetOrNull<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${itemId}`),
    enabled: kind === "channel" && !isCatchup,
  });

  const currentEpisode = seriesQuery.data?.episodes.find((e) => e.id === itemId);
  const nextEpisode = useMemo(
    () => (currentEpisode ? findNextEpisode(seriesQuery.data?.episodes ?? [], currentEpisode) : undefined),
    [seriesQuery.data, currentEpisode],
  );
  const currentChannel = channelsQuery.data?.items.find((c) => c.id === itemId);

  const title =
    kind === "movie"
      ? movieQuery.data?.clean_title
      : kind === "episode"
        ? seriesQuery.data?.clean_title
        : currentChannel?.display_name;
  const subtitle =
    kind === "episode" && currentEpisode
      ? `T${currentEpisode.season} · ${currentEpisode.number}. ${currentEpisode.title ?? `Episodio ${currentEpisode.number}`}`
      : kind === "channel"
        ? (channelEpgQuery.data?.[0]?.current?.title ?? null)
        : null;

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
      ? `/stream/live-buffer/channel/${itemId}?profile_id=${profileId}`
      : `/stream/${kind}/${itemId}?profile_id=${profileId}${search.source ? `&source=${search.source}` : ""}&passthrough=true`;

  const ready = base != null && profileId !== null && startSeconds !== null && (title != null || titleWaitOver);
  const playedPathRef = useRef<string | null>(null);

  const play = useCallback(() => {
    if (!base || startSeconds === null) return;
    const request: NativePlayRequest = {
      url: `${base}${streamPath}`,
      title: title ?? "",
      subtitle,
      start_seconds: startSeconds,
      live: kind === "channel" && !isCatchup,
      live_buffer: liveBuffer,
    };
    setError(null);
    invoke("native_player_play", { request }).catch((err: unknown) => setError(errorMessage(err)));
  }, [base, streamPath, title, subtitle, startSeconds, kind, isCatchup, liveBuffer]);

  useEffect(() => {
    if (!ready || playedPathRef.current === streamPath) return;
    playedPathRef.current = streamPath;
    play();
  }, [ready, streamPath, play]);

  const goBack = useCallback(() => {
    if (kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: id } });
    else if (kind === "episode" && search.seriesId) navigate({ to: "/series/$seriesId", params: { seriesId: search.seriesId } });
    else navigate({ to: isCatchup ? "/guide" : "/live" });
  }, [kind, id, search.seriesId, isCatchup, navigate]);

  // Los eventos llegan a un solo oyente; lo que cambia de un render a otro se lee de aquí.
  const latestRef = useRef({ kind, itemId, profileId, nextEpisode, channels: channelsQuery.data?.items, search, goBack });
  useEffect(() => {
    latestRef.current = { kind, itemId, profileId, nextEpisode, channels: channelsQuery.data?.items, search, goBack };
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
          // Suelta ya la conexión con el proveedor: la grabación del directo no tiene que
          // esperar a quedarse sin lectores.
          if (latest.kind === "channel") apiDelete("/api/live-buffer").catch(() => {});
          if (payload.ended && latest.kind === "episode" && latest.nextEpisode) {
            navigate({
              to: "/player/$kind/$id",
              params: { kind: "episode", id: String(latest.nextEpisode.id) },
              search: { seriesId: latest.search.seriesId },
            });
          } else {
            latest.goBack();
          }
          break;
        }
        case "zap": {
          const items = latest.channels ?? [];
          const index = items.findIndex((c) => c.id === latest.itemId);
          const next = index === -1 ? undefined : items[index + payload.direction];
          if (next) {
            navigate({
              to: "/player/$kind/$id",
              params: { kind: "channel", id: String(next.id) },
              search: { categoryId: latest.search.categoryId },
            });
          }
          break;
        }
        case "error":
          setError(payload.message);
          break;
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
