import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGetOrNull, apiPost, useApiBase } from "../../lib/api-client";
import type {
  ChannelsPage,
  EpisodeSummary,
  MovieDetailResponse,
  PlaybackProgressResponse,
  PlaybackTierResponse,
  SeriesDetailResponse,
} from "../../lib/api-types";
import { ArrowDownIcon, ArrowUpIcon, FullscreenIcon, MuteIcon, PauseIcon, PlayIcon, VolumeIcon } from "../../components/icons";

type Kind = "channel" | "movie" | "episode";

const CONTROLS_HIDE_DELAY_MS = 3_000;
const PROGRESS_SAVE_INTERVAL_MS = 10_000;
const NEXT_EPISODE_TRIGGER_SECONDS = 15;
const NEXT_EPISODE_COUNTDOWN_SECONDS = 10;
const RESUME_MIN_SECONDS = 5;
const SEEK_STEP_SECONDS = 10;

/** Cascada de reproducción de F6: un `<video>` nativo apuntando a `/stream/:kind/:id`.
 *
 * El nivel 1 (MP4 progresivo con Range) es de verdad byte-seekable. El nivel 2 (remux
 * en vivo, sin longitud conocida) no lo es: buscar significa pedir un stream nuevo con
 * `?start=<segundos>`, que `media::ffmpeg` traduce en un `-ss` de entrada.
 *
 * **Por qué no se usa `video.duration` para distinguir los dos casos** (como en una
 * versión anterior de este fichero): un fMP4 fragmentado servido por `<video src>` sin
 * MSE puede dar una duración finita pero *falsa* mientras llegan los primeros
 * fragmentos (una caída real, encontrada probando contra el proveedor real del
 * usuario — la barra de búsqueda mostraba "0:01" con la película en el segundo 7). El
 * nivel se pide al núcleo (`GET /api/playback/tier`, ya cacheado por la sonda que hizo
 * el propio `/stream/*` al arrancar) y es la única fuente de verdad de si se puede
 * buscar de forma nativa. */
export function PlayerScreen() {
  const { kind, id } = useParams({ from: "/player/$kind/$id" }) as { kind: Kind; id: string };
  const search = useSearch({ from: "/player/$kind/$id" });
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const itemId = Number(id);

  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hideTimerRef = useRef<number | undefined>(undefined);
  const mountedAtRef = useRef(performance.now());
  const firstFrameMsRef = useRef<number | null>(null);

  const [playing, setPlaying] = useState(true);
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [statsVisible, setStatsVisible] = useState(false);
  const [videoCurrentTime, setVideoCurrentTime] = useState(0);
  const [appliedStart, setAppliedStart] = useState(0);
  const [resumeReady, setResumeReady] = useState(kind === "channel"); // el directo no espera a nada.
  const [resumeBanner, setResumeBanner] = useState<number | null>(null);
  const [nextCountdown, setNextCountdown] = useState<number | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const movieQuery = useQuery({
    queryKey: ["player-movie", itemId],
    queryFn: () => apiGetOrNull<MovieDetailResponse>(`/api/movies/${itemId}`),
    enabled: kind === "movie",
  });
  const seriesQuery = useQuery({
    queryKey: ["player-series", search.seriesId],
    queryFn: () => apiGetOrNull<SeriesDetailResponse>(`/api/series/${search.seriesId}`),
    enabled: kind === "episode" && Boolean(search.seriesId),
  });
  const channelsQuery = useQuery({
    queryKey: ["player-channels", search.categoryId],
    queryFn: () =>
      apiGetOrNull<ChannelsPage>(`/api/live/channels?limit=200${search.categoryId ? `&category_id=${search.categoryId}` : ""}`),
    enabled: kind === "channel",
  });
  // Se pide siempre (no solo con el panel de estadísticas abierto) en VOD: es la fuente
  // más fiable de la duración real del fichero (ffprobe), mejor que `runtime_min` de
  // TMDB, que es un dato editorial y a menudo ni siquiera está. En directo no hace
  // falta: no hay duración que mostrar.
  const tierQuery = useQuery({
    queryKey: ["player-tier", kind, itemId],
    queryFn: () => apiGetOrNull<PlaybackTierResponse>(`/api/playback/tier/${kind}/${itemId}`),
    enabled: kind !== "channel",
    // La primera petición puede llegar antes de que el propio /stream/* termine de
    // sondear (van en paralelo al montar) y encontrar la caché todavía vacía. En vez de
    // depender solo del refetch de `onLoadedMetadata` (que asume que loadedmetadata
    // llega después de terminar el sondeo, no siempre demostrado), reintenta cada
    // segundo mientras no se sepa el nivel; en cuanto se sabe, para de sondear salvo que
    // el panel de estadísticas esté abierto (ahí sigue cada 3 s para verlo en vivo).
    refetchInterval: (query) => {
      if (query.state.data?.tier == null) return 1000;
      return statsVisible ? 3000 : false;
    },
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
        ? `${seriesQuery.data?.clean_title ?? ""}${currentEpisode ? ` · ${currentEpisode.number}. ${currentEpisode.title ?? "Episodio " + currentEpisode.number}` : ""}`
        : currentChannel?.display_name;

  // La duración real del fichero (ffprobe, vía tierQuery) manda sobre la editorial de
  // TMDB/el proveedor: es la única correcta para un stream remuxeado (nivel 2), donde el
  // propio <video> nunca la conoce por sí mismo.
  const editorialDurationSeconds =
    kind === "movie" ? (movieQuery.data?.runtime_min ? movieQuery.data.runtime_min * 60 : null) : kind === "episode" ? (currentEpisode?.duration_s ?? null) : null;
  const metadataDurationSeconds = tierQuery.data?.duration_s ?? editorialDurationSeconds;

  // Cargar el progreso guardado antes de montar el <video> en VOD: decide si hay que
  // arrancar con `?start=` (nivel 2) — para directo no hay nada que resumir.
  useEffect(() => {
    if (kind === "channel") return;
    let cancelled = false;
    apiGetOrNull<PlaybackProgressResponse>(`/api/playback/progress?item_kind=${kind}&item_id=${itemId}`).then((progress) => {
      if (cancelled) return;
      if (progress && !progress.finished && progress.position_s > RESUME_MIN_SECONDS) {
        setAppliedStart(progress.position_s);
        setResumeBanner(progress.position_s);
      }
      setResumeReady(true);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, itemId]);

  useEffect(() => {
    if (resumeBanner == null) return;
    const timeout = setTimeout(() => setResumeBanner(null), 4000);
    return () => clearTimeout(timeout);
  }, [resumeBanner]);

  // El nivel decide si se puede buscar de forma nativa; `video.duration` no es fiable
  // para esto (ver comentario de cabecera). `metadataDurationSeconds` (ffprobe) es la
  // única fuente de duración que se usa: la del propio `<video>` puede ser una cifra de
  // un fragmento suelto mientras el fMP4 remuxeado sigue llegando.
  const nativeSeekable = tierQuery.data?.tier === 1;
  const displayDuration = metadataDurationSeconds;
  const displayCurrentTime = nativeSeekable ? videoCurrentTime : appliedStart + videoCurrentTime;

  const streamPath =
    kind === "channel" ? `/stream/channel/${itemId}` : `/stream/${kind}/${itemId}${appliedStart > 0 ? `?start=${appliedStart}` : ""}`;
  const streamUrl = base ? `${base}${streamPath}` : undefined;

  // `saveProgress` necesita ser estable de verdad: si cambiara en cada `timeupdate` (como
  // pasaría si dependiera directamente de `displayCurrentTime`), el intervalo de guardado
  // de abajo se reiniciaría antes de completar sus 10 s y nunca llegaría a disparar. Por
  // eso lee los valores actuales de una ref en vez de cerrarlos por clausura.
  const latestRef = useRef({ kind, itemId, displayCurrentTime, displayDuration, resumeReady });
  useEffect(() => {
    latestRef.current = { kind, itemId, displayCurrentTime, displayDuration, resumeReady };
  });

  const saveProgress = useCallback((finishedHint?: boolean) => {
    const { kind, itemId, displayCurrentTime, displayDuration, resumeReady } = latestRef.current;
    if (kind === "channel" || !resumeReady) return;
    const position = Math.floor(displayCurrentTime);
    if (position <= 0 && !finishedHint) return;
    apiPost("/api/playback/progress", {
      item_kind: kind,
      item_id: itemId,
      position_s: position,
      duration_s: displayDuration ? Math.floor(displayDuration) : null,
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (kind === "channel") return;
    const interval = window.setInterval(() => saveProgress(), PROGRESS_SAVE_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [kind, saveProgress]);

  useEffect(() => {
    return () => saveProgress();
  }, [saveProgress]);

  // Siguiente episodio con cuenta atrás cancelable, en los últimos segundos.
  useEffect(() => {
    if (kind !== "episode" || !nextEpisode || nextCountdown !== null) return;
    if (displayDuration && displayCurrentTime > 0 && displayDuration - displayCurrentTime <= NEXT_EPISODE_TRIGGER_SECONDS) {
      setNextCountdown(NEXT_EPISODE_COUNTDOWN_SECONDS);
    }
  }, [kind, nextEpisode, displayDuration, displayCurrentTime, nextCountdown]);

  useEffect(() => {
    if (nextCountdown === null) return;
    if (nextCountdown <= 0) {
      saveProgress(true);
      navigate({ to: "/player/$kind/$id", params: { kind: "episode", id: String(nextEpisode!.id) }, search: { seriesId: search.seriesId } });
      return;
    }
    const timeout = window.setTimeout(() => setNextCountdown((c) => (c ?? 1) - 1), 1000);
    return () => window.clearTimeout(timeout);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nextCountdown]);

  const resetHideTimer = useCallback(() => {
    setControlsVisible(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => setControlsVisible(false), CONTROLS_HIDE_DELAY_MS);
  }, []);

  useEffect(() => {
    resetHideTimer();
    return () => {
      if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    };
  }, [resetHideTimer]);

  const goBack = useCallback(() => {
    saveProgress();
    if (kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: id } });
    else if (kind === "episode" && search.seriesId) navigate({ to: "/series/$seriesId", params: { seriesId: search.seriesId } });
    else navigate({ to: "/live" });
  }, [kind, id, search.seriesId, navigate, saveProgress]);

  const zapChannel = useCallback(
    (direction: 1 | -1) => {
      const items = channelsQuery.data?.items ?? [];
      const idx = items.findIndex((c) => c.id === itemId);
      const next = idx === -1 ? undefined : items[idx + direction];
      if (!next) return;
      navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(next.id) }, search: { categoryId: search.categoryId } });
    },
    [channelsQuery.data, itemId, navigate, search.categoryId],
  );

  const seekTo = useCallback(
    (targetSeconds: number) => {
      const clamped = Math.max(0, displayDuration ? Math.min(targetSeconds, displayDuration - 1) : targetSeconds);
      if (nativeSeekable && videoRef.current) {
        videoRef.current.currentTime = clamped;
      } else {
        setAppliedStart(Math.floor(clamped));
      }
    },
    [displayDuration, nativeSeekable],
  );

  const togglePlay = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) video.play().catch(() => {});
    else video.pause();
  }, []);

  const toggleMute = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
    setMuted(video.muted);
  }, []);

  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else containerRef.current?.requestFullscreen().catch(() => {});
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        setStatsVisible((v) => !v);
        return;
      }
      switch (e.key) {
        case "Escape":
          e.preventDefault();
          goBack();
          break;
        case " ":
        case "k":
          e.preventDefault();
          togglePlay();
          break;
        case "m":
          toggleMute();
          break;
        case "f":
          toggleFullscreen();
          break;
        case "ArrowUp":
          e.preventDefault();
          if (kind === "channel") zapChannel(-1);
          else setVolume((v) => Math.min(1, v + 0.1));
          break;
        case "ArrowDown":
          e.preventDefault();
          if (kind === "channel") zapChannel(1);
          else setVolume((v) => Math.max(0, v - 0.1));
          break;
        case "ArrowLeft":
          if (kind !== "channel") seekTo(displayCurrentTime - SEEK_STEP_SECONDS);
          break;
        case "ArrowRight":
          if (kind !== "channel") seekTo(displayCurrentTime + SEEK_STEP_SECONDS);
          break;
      }
      resetHideTimer();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [kind, goBack, togglePlay, toggleMute, toggleFullscreen, zapChannel, seekTo, displayCurrentTime, resetHideTimer]);

  useEffect(() => {
    const video = videoRef.current;
    if (video) video.volume = volume;
  }, [volume, streamPath]);

  const canRenderVideo = streamUrl && resumeReady;

  return (
    <div
      ref={containerRef}
      className="relative flex h-full w-full items-center justify-center bg-black outline-none"
      onMouseMove={resetHideTimer}
      tabIndex={-1}
    >
      {canRenderVideo ? (
        <video
          key={streamPath}
          ref={videoRef}
          src={streamUrl}
          autoPlay
          muted={muted}
          className="h-full w-full"
          onLoadedMetadata={(e) => {
            const video = e.currentTarget;
            // El arranque siempre manda `?start=` por si el nivel resultara ser 2 (ver
            // comentario de cabecera); si el navegador demuestra que este recurso sí
            // admite Range de verdad (`seekable` no vacío — más fiable que mirar
            // `duration`, que puede ser una cifra provisional en fMP4 fragmentado), se
            // corrige con una búsqueda nativa real en vez de confiar en el `?start=`
            // que el proxy de nivel 1 ignora.
            if (video.seekable.length > 0 && appliedStart > 0 && video.currentTime < appliedStart - 1) {
              video.currentTime = appliedStart;
            }
            // `tierQuery` puede haberse pedido antes de que este mismo stream terminara
            // de sondear el códec (van en paralelo al montar): al llegar hasta aquí, la
            // sonda ya ha terminado seguro, así que ahora sí va a acertar con el nivel y
            // la duración reales si la primera vez llegaron tarde.
            if (kind !== "channel") tierQuery.refetch();
          }}
          onTimeUpdate={(e) => setVideoCurrentTime(e.currentTarget.currentTime)}
          onPlaying={() => {
            if (firstFrameMsRef.current == null) firstFrameMsRef.current = Math.round(performance.now() - mountedAtRef.current);
          }}
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onVolumeChange={(e) => setMuted(e.currentTarget.muted)}
          onError={() => setErrorMessage("No se pudo reproducir este contenido. Comprueba tu conexión o el estado de la cuenta.")}
        />
      ) : (
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-neutral-700 border-t-red-600" aria-label="Cargando" />
      )}

      {resumeBanner != null && (
        <div className="absolute left-6 top-6 rounded-lg bg-black/70 px-4 py-2 text-sm text-neutral-200">
          Reanudando desde {formatTime(resumeBanner)}
        </div>
      )}

      {errorMessage && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/80 px-10 text-center text-neutral-200">{errorMessage}</div>
      )}

      {nextCountdown !== null && nextEpisode && (
        <div className="absolute bottom-28 right-6 flex flex-col gap-2 rounded-lg bg-neutral-900/95 p-4 text-sm text-neutral-200 shadow-lg">
          <p>
            Siguiente: {nextEpisode.number}. {nextEpisode.title || `Episodio ${nextEpisode.number}`} en {nextCountdown}s
          </p>
          <button type="button" onClick={() => setNextCountdown(null)} className="self-start rounded bg-neutral-800 px-3 py-1 outline-none hover:bg-neutral-700">
            Cancelar
          </button>
        </div>
      )}

      {statsVisible && (
        <div className="absolute right-6 top-6 w-72 rounded-lg bg-black/80 p-4 font-mono text-xs text-neutral-200">
          <p className="mb-2 font-semibold text-neutral-100">Estadísticas (Ctrl+Shift+D)</p>
          <StatRow label="Nivel" value={tierQuery.data?.tier != null ? String(tierQuery.data.tier) : "desconocido"} />
          <StatRow label="Vídeo" value={tierQuery.data?.video_codec ?? "—"} />
          <StatRow label="Audio" value={tierQuery.data?.audio_codec ?? "—"} />
          <StatRow label="Contenedor" value={tierQuery.data?.container ?? "—"} />
          <StatRow label="Resolución" value={videoRef.current ? `${videoRef.current.videoWidth}×${videoRef.current.videoHeight}` : "—"} />
          <StatRow label="1er fotograma" value={firstFrameMsRef.current != null ? `${firstFrameMsRef.current} ms` : "—"} />
          <StatRow label="Buffer" value={`${bufferedSeconds(videoRef.current).toFixed(1)} s`} />
        </div>
      )}

      <div
        className={`absolute inset-x-0 bottom-0 flex flex-col gap-3 bg-gradient-to-t from-black/90 to-transparent px-6 pb-5 pt-16 transition-opacity duration-200 ${
          controlsVisible ? "opacity-100" : "pointer-events-none opacity-0"
        }`}
      >
        {title && <p className="truncate text-lg font-medium text-white">{title}</p>}

        {kind !== "channel" && displayDuration ? (
          <input
            type="range"
            min={0}
            max={Math.max(displayDuration, 1)}
            value={Math.min(displayCurrentTime, displayDuration)}
            onChange={(e) => seekTo(Number(e.target.value))}
            className="w-full accent-red-600"
            aria-label="Progreso"
          />
        ) : null}

        <div className="flex items-center gap-4 text-white">
          <button type="button" onClick={togglePlay} className="outline-none" aria-label={playing ? "Pausar" : "Reproducir"}>
            {playing ? <PauseIcon className="h-7 w-7" /> : <PlayIcon className="h-7 w-7" />}
          </button>

          {kind !== "channel" && (
            <div className="flex items-center gap-2 text-sm">
              <button
                type="button"
                onClick={() => seekTo(displayCurrentTime - SEEK_STEP_SECONDS)}
                className="rounded px-2 py-1 outline-none hover:bg-white/10"
                title="Retroceder 10 s (flecha izquierda)"
              >
                «10s
              </button>
              <button
                type="button"
                onClick={() => seekTo(displayCurrentTime + SEEK_STEP_SECONDS)}
                className="rounded px-2 py-1 outline-none hover:bg-white/10"
                title="Avanzar 10 s (flecha derecha)"
              >
                10s»
              </button>
            </div>
          )}

          {kind === "channel" ? (
            <div className="flex items-center gap-2">
              <button type="button" onClick={() => zapChannel(-1)} className="outline-none" aria-label="Canal anterior">
                <ArrowUpIcon className="h-5 w-5" />
              </button>
              <button type="button" onClick={() => zapChannel(1)} className="outline-none" aria-label="Canal siguiente">
                <ArrowDownIcon className="h-5 w-5" />
              </button>
            </div>
          ) : (
            <span className="w-24 shrink-0 text-sm tabular-nums text-neutral-300">
              {formatTime(displayCurrentTime)} {displayDuration ? `/ ${formatTime(displayDuration)}` : ""}
            </span>
          )}

          <button type="button" onClick={toggleMute} className="outline-none" aria-label={muted ? "Quitar silencio" : "Silenciar"}>
            {muted || volume === 0 ? <MuteIcon className="h-5 w-5" /> : <VolumeIcon className="h-5 w-5" />}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={muted ? 0 : volume}
            onChange={(e) => {
              setVolume(Number(e.target.value));
              setMuted(false);
            }}
            className="w-24 accent-red-600"
            aria-label="Volumen"
          />

          <button type="button" onClick={goBack} className="ml-auto text-sm text-neutral-300 outline-none hover:text-white">
            ← Volver (Esc)
          </button>
          <button type="button" onClick={toggleFullscreen} className="outline-none" aria-label="Pantalla completa">
            <FullscreenIcon className="h-5 w-5" />
          </button>
        </div>
      </div>
    </div>
  );
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4 py-0.5">
      <span className="text-neutral-500">{label}</span>
      <span className="truncate text-neutral-200">{value}</span>
    </div>
  );
}

function findNextEpisode(all: EpisodeSummary[], current: EpisodeSummary): EpisodeSummary | undefined {
  const sameSeason = all.filter((e) => e.season === current.season).sort((a, b) => a.number - b.number);
  const inSeasonNext = sameSeason.find((e) => e.number > current.number);
  if (inSeasonNext) return inSeasonNext;

  const seasons = [...new Set(all.map((e) => e.season))].sort((a, b) => a - b);
  const nextSeason = seasons.find((s) => s > current.season);
  if (nextSeason == null) return undefined;
  return all.filter((e) => e.season === nextSeason).sort((a, b) => a.number - b.number)[0];
}

function bufferedSeconds(video: HTMLVideoElement | null): number {
  if (!video || video.buffered.length === 0) return 0;
  return video.buffered.end(video.buffered.length - 1) - video.currentTime;
}

function formatTime(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = String(m).padStart(h > 0 ? 2 : 1, "0");
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
