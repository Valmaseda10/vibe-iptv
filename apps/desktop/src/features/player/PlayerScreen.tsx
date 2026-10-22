import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { failoverNotice, nextUntriedSource, sourceLabel } from "./movieSources";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { apiGetOrNull, apiPost, useApiBase } from "../../lib/api-client";
import { AUDIO_PASSTHROUGH } from "../../lib/codecSupport";
import { useActiveProfile } from "../../lib/activeProfile";
import type {
  ChannelSummary,
  ChannelsPage,
  EpgNowNextResponse,
  MovieDetailResponse,
  PlaybackProgressResponse,
  PlaybackTierResponse,
  StreamFailure,
  SeriesDetailResponse,
  SubtitleCuesResponse,
} from "../../lib/api-types";
import { ArrowDownIcon, ArrowUpIcon, BackIcon, FullscreenIcon, MuteIcon, PauseIcon, PictureInPictureIcon, PlayIcon, VolumeIcon } from "../../components/icons";
import { useLiveTimeshift } from "./useLiveTimeshift";
import { SUBTITLE_SIZES, subtitleCueCss, useSubtitleStyle } from "../../lib/subtitleStyle";
import { findNextEpisode } from "./episodes";

type Kind = "channel" | "movie" | "episode";

const CONTROLS_HIDE_DELAY_MS = 3_000;
const PROGRESS_SAVE_INTERVAL_MS = 10_000;
const NEXT_EPISODE_TRIGGER_SECONDS = 15;
const NEXT_EPISODE_COUNTDOWN_SECONDS = 10;
const RESUME_MIN_SECONDS = 5;
const SEEK_STEP_SECONDS = 10;
/** En el nivel 2 cada salto reabre el remux (ffmpeg y una petición al proveedor): se
 * espera a que el usuario pare de arrastrar la barra o de pulsar antes de saltar, y los
 * saltos seguidos se suman. */
const SEEK_SETTLE_MS = 450;
/** Paso de volumen con la rueda del ratón y con + / −. */
const VOLUME_STEP = 0.05;
const VOLUME_STORAGE_KEY = "vibe.player.volume";

/** Volumen de la última vez (preferencia de este equipo); 1 si no hay o no se puede leer. */
function readSavedVolume(): number {
  try {
    const saved = window.localStorage.getItem(VOLUME_STORAGE_KEY);
    const value = saved == null ? NaN : Number(saved);
    return Number.isFinite(value) && value >= 0 && value <= 1 ? value : 1;
  } catch {
    return 1;
  }
}
/** Velocidades de reproducción del VOD (F14). */
const PLAYBACK_RATES = [0.5, 0.75, 1, 1.25, 1.5, 2];
/** Un directo IPTV se corta con normalidad (el proveedor tiene un mal minuto, un salto
 * de red del propio PC...); reintentar solo, unas pocas veces, es la diferencia entre
 * "la app está rota" y "el proveedor tuvo un mal minuto". El VOD no reintenta solo: un
 * fallo ahí suele ser real (código no soportado, fichero roto), no un corte pasajero. */
const MAX_RECONNECT_ATTEMPTS = 3;
const RECONNECT_DELAY_MS = 2_000;
/** Códigos que no se arreglan reintentando: se enseña el error real de inmediato en vez
 * de gastar los reintentos en algo que va a fallar igual las tres veces. */
const DEFINITIVE_ERROR_STATUSES = new Set([400, 402, 403, 404, 501, 503]);
/** Cada cuánto se piden las frases nuevas de subtítulos. Es el núcleo local leyendo un
 * fichero, no el proveedor: puede ser frecuente. */
const SUBTITLE_POLL_MS = 1_500;
/** Tras la última cifra tecleada, cuánto se espera antes de cambiar de canal (como en un
 * mando: "1", "2" → canal 12). */
const NUMBER_ENTRY_MS = 1_500;
/** Línea de los subtítulos contando desde abajo: despegados del borde, y por encima de
 * la barra de controles mientras se ve (si no, la tapa). */
const SUBTITLE_LINE_HIDDEN_CONTROLS = -2;
const SUBTITLE_LINE_VISIBLE_CONTROLS = -4;

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
  const { profileId } = useActiveProfile();
  const itemId = Number(id);

  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hideTimerRef = useRef<number | undefined>(undefined);
  const mountedAtRef = useRef(performance.now());
  const firstFrameMsRef = useRef<number | null>(null);

  const [playing, setPlaying] = useState(true);
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(readSavedVolume);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [statsVisible, setStatsVisible] = useState(false);
  const [videoCurrentTime, setVideoCurrentTime] = useState(0);
  const [appliedStart, setAppliedStart] = useState(0);
  const [resumeReady, setResumeReady] = useState(kind === "channel"); // el directo no espera a nada.
  const [resumeBanner, setResumeBanner] = useState<number | null>(null);
  const [channelOverlayVisible, setChannelOverlayVisible] = useState(true);
  const [nextCountdown, setNextCountdown] = useState<number | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [needsMpv, setNeedsMpv] = useState(false);
  const [mpvLaunching, setMpvLaunching] = useState(false);
  const [playingInMpv, setPlayingInMpv] = useState(false);
  const [buffering, setBuffering] = useState(false);
  const [reconnectAttempt, setReconnectAttempt] = useState(0);
  const [retryNonce, setRetryNonce] = useState(0);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [inPictureInPicture, setInPictureInPicture] = useState(false);
  const reconnectTimerRef = useRef<number | undefined>(undefined);
  // Pista de subtítulos elegida, ligada al ítem: al pasar al siguiente episodio no se
  // arrastra un índice que en ese fichero puede ser otro idioma (o no existir).
  const [subtitleChoice, setSubtitleChoice] = useState<{ item: string; track: number } | null>(null);
  // Aspecto de los subtítulos del perfil (Ajustes → Subtítulos).
  const [subtitleStyle, setSubtitleStyle] = useSubtitleStyle();
  const subtitleTrack = kind !== "channel" && subtitleChoice?.item === `${kind}:${itemId}` ? subtitleChoice.track : null;

  const movieQuery = useQuery({
    queryKey: ["player-movie", itemId, profileId],
    queryFn: () => apiGetOrNull<MovieDetailResponse>(`/api/movies/${itemId}?profile_id=${profileId}`),
    enabled: kind === "movie",
  });
  const seriesQuery = useQuery({
    queryKey: ["player-series", search.seriesId, profileId],
    queryFn: () => apiGetOrNull<SeriesDetailResponse>(`/api/series/${search.seriesId}?profile_id=${profileId}`),
    enabled: kind === "episode" && Boolean(search.seriesId) && profileId !== null,
  });
  const channelEpgQuery = useQuery({
    queryKey: ["player-epg-now", itemId],
    queryFn: () => apiGetOrNull<EpgNowNextResponse[]>(`/api/epg/now?channel_ids=${itemId}`),
    enabled: kind === "channel",
  });
  // Los canales de alrededor del que se ve (nombre, lista y zapping), no la primera página:
  // en listas de miles el canal podía no estar en ella. La ventana da la vuelta por los
  // extremos y se recentra en cada cambio de canal; mientras llega la nueva, sigue la vieja.
  const channelsQuery = useQuery({
    queryKey: ["player-channels", profileId, search.categoryId, search.favorites, itemId],
    queryFn: () =>
      apiGetOrNull<ChannelsPage>(
        `/api/live/channels?profile_id=${profileId}&limit=100&around=${itemId}${search.favorites ? "&favorites=1" : search.categoryId ? `&category_id=${search.categoryId}` : ""}`,
      ),
    placeholderData: keepPreviousData,
    enabled: kind === "channel" && profileId !== null,
  });
  // Se pide siempre (no solo con el panel de estadísticas abierto) en VOD: es la fuente
  // más fiable de la duración real del fichero (ffprobe), mejor que `runtime_min` de
  // TMDB, que es un dato editorial y a menudo ni siquiera está. En directo no hace
  // falta: no hay duración que mostrar.
  const tierQuery = useQuery({
    queryKey: ["player-tier", kind, itemId, search.source],
    queryFn: () => apiGetOrNull<PlaybackTierResponse>(`/api/playback/tier/${kind}/${itemId}${search.source ? `?source=${search.source}` : ""}`),
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
    if (kind === "channel" || profileId === null) return;
    let cancelled = false;
    apiGetOrNull<PlaybackProgressResponse>(`/api/playback/progress?profile_id=${profileId}&item_kind=${kind}&item_id=${itemId}`).then((progress) => {
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
  }, [kind, itemId, profileId]);

  useEffect(() => {
    if (resumeBanner == null) return;
    const timeout = setTimeout(() => setResumeBanner(null), 4000);
    return () => clearTimeout(timeout);
  }, [resumeBanner]);

  // Superposición de "qué están dando" al cambiar de canal (F7): se enseña al entrar y
  // en cada zapping, y se retira sola a los pocos segundos.
  useEffect(() => {
    if (kind !== "channel") return;
    setChannelOverlayVisible(true);
    const timeout = setTimeout(() => setChannelOverlayVisible(false), 5000);
    return () => clearTimeout(timeout);
  }, [kind, itemId]);

  // El nivel decide si se puede buscar de forma nativa; `video.duration` no es fiable
  // para esto (ver comentario de cabecera). `metadataDurationSeconds` (ffprobe) es la
  // única fuente de duración que se usa: la del propio `<video>` puede ser una cifra de
  // un fragmento suelto mientras el fMP4 remuxeado sigue llegando.
  // Un directo nunca se busca de forma nativa, aunque llegue por HLS (nivel 1). Con
  // subtítulos tampoco: salen del remux, así que el núcleo sirve el nivel 2 aunque el
  // fichero admitiera el 1.
  const nativeSeekable = kind !== "channel" && tierQuery.data?.tier === 1 && subtitleTrack == null;
  const displayDuration = metadataDurationSeconds;
  const displayCurrentTime = nativeSeekable ? videoCurrentTime : appliedStart + videoCurrentTime;
  // "Saltar intro" (F14): solo con un capítulo de intro en el fichero, nunca adivinado.
  const introStart = tierQuery.data?.intro_start_s;
  const introEnd = tierQuery.data?.intro_end_s;
  const inIntro =
    kind !== "channel" && introStart != null && introEnd != null && displayCurrentTime >= introStart && displayCurrentTime < introEnd - 1;

  // Catch-up (F9): la Guía manda aquí en vez de al directo cuando el programa pulsado ya
  // pasó. Solo se distingue por la presencia de `catchupStart` en la URL de la ruta, no
  // por un `kind` aparte: sigue siendo el mismo canal, solo cambia a qué momento apunta.
  const isCatchup = kind === "channel" && search.catchupStart !== undefined;
  // Pausar y rebobinar el directo (F13): ver `useLiveTimeshift`.
  const liveTimeshift = kind === "channel" && !isCatchup;

  // Teclear un número en el directo cambia a ese canal; "0" solo vuelve al anterior (F13).
  const [typedNumber, setTypedNumber] = useState("");
  const [numberMessage, setNumberMessage] = useState<string | null>(null);
  const previousChannelRef = useRef<number | null>(null);
  const currentChannelRef = useRef<number | null>(null);
  useEffect(() => {
    if (kind !== "channel") return;
    if (currentChannelRef.current != null && currentChannelRef.current !== itemId) previousChannelRef.current = currentChannelRef.current;
    currentChannelRef.current = itemId;
  }, [kind, itemId]);
  useEffect(() => {
    if (!typedNumber) return;
    const timer = window.setTimeout(async () => {
      const typed = typedNumber;
      setTypedNumber("");
      if (typed === "0") {
        const previous = previousChannelRef.current;
        if (previous == null) setNumberMessage("No hay canal anterior");
        else navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(previous) }, search: {} });
        return;
      }
      const channel = await apiGetOrNull<ChannelSummary>(`/api/live/channels/by-number/${Number(typed)}?profile_id=${profileId}&near=${itemId}`).catch(() => null);
      if (channel) navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(channel.id) }, search: {} });
      else setNumberMessage(`No hay canal ${Number(typed)}`);
    }, NUMBER_ENTRY_MS);
    return () => window.clearTimeout(timer);
  }, [typedNumber, navigate, profileId, itemId]);
  useEffect(() => {
    if (!numberMessage) return;
    const timer = window.setTimeout(() => setNumberMessage(null), 2_000);
    return () => window.clearTimeout(timer);
  }, [numberMessage]);
  const timeshift = useLiveTimeshift({ enabled: liveTimeshift, channelId: itemId, base, videoRef });
  // Si la grabación falla, se vuelve al directo (otra sesión, sin lo grabado).
  useEffect(() => {
    if (timeshift.failed && timeshift.inBuffer) timeshift.leaveBuffer();
  }, [timeshift]);
  // El audio que la WebView decodifica ella misma no se convierte en el núcleo.
  const passthrough = AUDIO_PASSTHROUGH ? `&audio_passthrough=${AUDIO_PASSTHROUGH}` : "";
  const streamPath = isCatchup
    ? `/stream/catchup/${itemId}?profile_id=${profileId}&start=${search.catchupStart}${search.catchupDuration ? `&duration_min=${search.catchupDuration}` : ""}${passthrough}`
    : kind === "channel"
      ? `/stream/channel/${itemId}?profile_id=${profileId}${passthrough}`
      : `/stream/${kind}/${itemId}?profile_id=${profileId}${appliedStart > 0 ? `&start=${appliedStart}` : ""}${search.source ? `&source=${search.source}` : ""}${search.audioTrack != null ? `&audio_track=${search.audioTrack}` : ""}${subtitleTrack != null ? `&subtitle_track=${subtitleTrack}` : ""}${passthrough}`;
  const streamUrl = base && profileId !== null ? `${base}${streamPath}` : undefined;

  // Cambiar de stream de verdad (zapping, o abrir otro título) es un intento nuevo:
  // sin esto, tres cortes seguidos en canales distintos agotarían los reintentos del
  // primero y dejarían el segundo sin ninguno. Un reintento ya programado era del
  // stream anterior: se cancela.
  useEffect(() => {
    setReconnectAttempt(0);
    setErrorMessage(null);
    setNeedsMpv(false);
    setPlayingInMpv(false);
    if (reconnectTimerRef.current) window.clearTimeout(reconnectTimerRef.current);
  }, [streamPath]);

  // Stream que se está viendo ahora mismo, para descartar el resultado de un fallo que
  // llega tarde, cuando el usuario ya cambió de canal.
  const currentStreamPathRef = useRef(streamPath);
  currentStreamPathRef.current = streamPath;

  useEffect(() => {
    return () => {
      if (reconnectTimerRef.current) window.clearTimeout(reconnectTimerRef.current);
    };
  }, []);

  // `<video onError>` no da el porqué (el navegador nunca ve el cuerpo de una respuesta
  // que no pudo interpretar como vídeo). El núcleo guarda el motivo del último fallo al
  // abrir cada stream y aquí se le pregunta (F9: mensajes que digan qué hacer). Antes se
  // volvía a pedir el propio stream para leer el error: eso abría otra sesión con el
  // proveedor, mataba la que se estaba viendo y, en un directo que funcionaba, la
  // respuesta no terminaba nunca y el reintento no llegaba a dispararse.
  //
  // En directo, antes de rendirse: un corte de proveedor es lo normal en IPTV, así que
  // se reintenta solo unas pocas veces (ver `MAX_RECONNECT_ATTEMPTS`) forzando un
  // remontaje del `<video>` con la misma URL, salvo que el motivo sea de los que no se
  // arreglan reintentando (`DEFINITIVE_ERROR_STATUSES`).
  // Fuentes de la película: se cambia a mano abajo y, si una no carga, se prueba sola la
  // siguiente desde el mismo punto (cada una una vez).
  const movieSources = useMemo(() => (kind === "movie" ? (movieQuery.data?.sources ?? []) : []), [kind, movieQuery.data]);
  const activeSourceId = search.source ?? movieSources[0]?.id ?? null;
  const activeSourceIndex = Math.max(0, movieSources.findIndex((source) => source.id === activeSourceId));
  const triedSourcesRef = useRef<Set<number>>(new Set());
  useEffect(() => {
    triedSourcesRef.current = new Set();
  }, [kind, itemId]);
  const [sourceNotice, setSourceNotice] = useState<string | null>(null);
  useEffect(() => {
    if (!sourceNotice) return;
    const timer = window.setTimeout(() => setSourceNotice(null), 5000);
    return () => window.clearTimeout(timer);
  }, [sourceNotice]);
  const switchSource = useCallback(
    (index: number, auto: boolean) => {
      const target = movieSources[index];
      if (!target || target.id === activeSourceId) return;
      if (auto) setSourceNotice(failoverNotice(activeSourceIndex, index));
      setAppliedStart(Math.floor(latestRef.current.displayCurrentTime));
      setVideoCurrentTime(0);
      navigate({ to: "/player/$kind/$id", params: { kind, id }, search: { ...search, source: target.id }, replace: true });
    },
    [movieSources, activeSourceId, activeSourceIndex, navigate, kind, id, search],
  );

  const handleVideoError = useCallback(async () => {
    if (!streamUrl) return;
    // En pausa, el núcleo deja de mandar el directo (sigue grabando): que se corte no es un
    // fallo. Y la grabación tiene sus propios errores (`useLiveTimeshift`).
    if (timeshift.inBuffer || timeshift.isPausedLive()) return;
    const failedPath = streamPath;
    let status: number | null = null;
    let text = "";
    try {
      const failure = await apiGetOrNull<StreamFailure>(`/api/playback/last-error/${isCatchup ? "catchup" : kind}/${itemId}`);
      if (failure) {
        status = failure.status;
        text = failure.message;
      }
    } catch {
      // Sin respuesta del propio núcleo: tampoco es un "no" definitivo, así que en
      // directo también merece un reintento.
    }
    if (currentStreamPathRef.current !== failedPath) return;

    // Película con más fuentes: antes de rendirse (o de abrir mpv), la siguiente sin probar.
    if (kind === "movie" && movieSources.length > 1) {
      if (activeSourceId != null) triedSourcesRef.current.add(activeSourceId);
      const nextIndex = nextUntriedSource(movieSources, triedSourcesRef.current);
      if (nextIndex >= 0) {
        switchSource(nextIndex, true);
        return;
      }
    }

    const definitive = status != null && DEFINITIVE_ERROR_STATUSES.has(status);
    if (kind === "channel" && !definitive && reconnectAttempt < MAX_RECONNECT_ATTEMPTS) {
      setReconnectAttempt((n) => n + 1);
      reconnectTimerRef.current = window.setTimeout(() => setRetryNonce((n) => n + 1), RECONNECT_DELAY_MS);
      return;
    }

    // "Abrir con mpv" lanza siempre el directo del canal (`/stream/mpv/:kind/:id`):
    // todavía no sabe pedir un bloque de catch-up, así que no se ofrece aquí para no
    // prometer algo que reproduciría el momento equivocado.
    setNeedsMpv(status === 501 && !isCatchup);
    setErrorMessage(text || "No se pudo reproducir este contenido.");
  }, [streamUrl, streamPath, isCatchup, kind, itemId, reconnectAttempt, timeshift, movieSources, activeSourceId, switchSource]);

  const launchMpv = useCallback(async () => {
    if (!base || profileId === null) return;
    setMpvLaunching(true);
    try {
      const response = await fetch(
        `${base}/stream/mpv/${kind}/${itemId}?profile_id=${profileId}${search.source ? `&source=${search.source}` : ""}`,
        { method: "POST" },
      );
      if (response.ok) {
        setErrorMessage(null);
        setNeedsMpv(false);
        setPlayingInMpv(true);
      } else {
        setErrorMessage((await response.text()) || "No se pudo abrir mpv.");
      }
    } catch {
      setErrorMessage("No se pudo abrir mpv.");
    } finally {
      setMpvLaunching(false);
    }
  }, [base, kind, itemId, profileId, search.source]);

  // Si el contenido necesita mpv (códec que este reproductor no abre), se abre solo: va
  // empaquetado con la app. El botón queda únicamente como reintento si falla. Una vez
  // por stream, para no relanzarlo en bucle.
  const autoMpvTriedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!needsMpv || autoMpvTriedFor.current === streamPath) return;
    autoMpvTriedFor.current = streamPath;
    void launchMpv();
  }, [needsMpv, streamPath, launchMpv]);

  // `saveProgress` necesita ser estable de verdad: si cambiara en cada `timeupdate` (como
  // pasaría si dependiera directamente de `displayCurrentTime`), el intervalo de guardado
  // de abajo se reiniciaría antes de completar sus 10 s y nunca llegaría a disparar. Por
  // eso lee los valores actuales de una ref en vez de cerrarlos por clausura.
  const latestRef = useRef({ kind, itemId, profileId, displayCurrentTime, displayDuration, resumeReady });
  useEffect(() => {
    latestRef.current = { kind, itemId, profileId, displayCurrentTime, displayDuration, resumeReady };
  });

  const saveProgress = useCallback((finishedHint?: boolean) => {
    const { kind, itemId, profileId, displayCurrentTime, displayDuration, resumeReady } = latestRef.current;
    if (kind === "channel" || !resumeReady || profileId === null) return;
    const position = Math.floor(displayCurrentTime);
    if (position <= 0 && !finishedHint) return;
    apiPost("/api/playback/progress", {
      profile_id: profileId,
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
    hideTimerRef.current = window.setTimeout(() => {
      // En pausa se quedan a la vista, como en Netflix.
      if (videoRef.current?.paused) return;
      setControlsVisible(false);
    }, CONTROLS_HIDE_DELAY_MS);
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
      // Tras el último canal viene el primero (y al revés), como en TiviMate y el iPhone.
      const next = idx === -1 || items.length < 2 ? undefined : items[(idx + direction + items.length) % items.length];
      if (!next) return;
      navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(next.id) }, search: { categoryId: search.categoryId, favorites: search.favorites } });
    },
    [channelsQuery.data, itemId, navigate, search.categoryId, search.favorites],
  );

  // Selector de pista de audio (F "premium", pendiente desde el plan original de F6).
  // Solo tiene efecto de verdad en nivel 2 (remux): cambiarlo reescribe `search`, lo que
  // cambia `streamPath` y por tanto la `key` del `<video>` más abajo — mismo mecanismo
  // que ya usa el selector de fuente/calidad de la ficha, no uno nuevo.
  const setAudioTrack = useCallback(
    (index: number) => {
      navigate({ to: "/player/$kind/$id", params: { kind, id }, search: { ...search, audioTrack: index } });
    },
    [kind, id, search, navigate],
  );

  // Activar o cambiar subtítulos reabre el remux (es él quien los extrae), así que se
  // retoma justo donde se estaba en vez de volver al último punto de arranque.
  const chooseSubtitles = useCallback(
    (track: number | null) => {
      setAppliedStart(Math.floor(latestRef.current.displayCurrentTime));
      setVideoCurrentTime(0);
      setSubtitleChoice(track == null ? null : { item: `${kind}:${itemId}`, track });
    },
    [kind, itemId],
  );

  // Las frases las va escribiendo el remux en un fichero según avanza; aquí se piden las
  // nuevas cada poco y se añaden al `<video>` como pista de texto. Sus tiempos ya están
  // en la escala del `<video>`: cero en el punto pedido con `?start=`, como el vídeo.
  const textTrackRef = useRef<TextTrack | null>(null);
  const subtitleLineRef = useRef(SUBTITLE_LINE_VISIBLE_CONTROLS);
  useEffect(() => {
    const video = videoRef.current;
    if (subtitleTrack == null || !video) return;
    const textTrack = video.addTextTrack("subtitles", "Subtítulos");
    textTrack.mode = "showing";
    textTrackRef.current = textTrack;
    let received = 0;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const response = await apiGetOrNull<SubtitleCuesResponse>(
          `/api/playback/subtitles/${kind}/${itemId}?track=${subtitleTrack}&start=${appliedStart}&from=${received}`,
        );
        if (cancelled) return;
        for (const cue of response?.cues ?? []) {
          const vttCue = new VTTCue(cue.start, cue.end, cue.text);
          vttCue.line = subtitleLineRef.current;
          textTrack.addCue(vttCue);
        }
        received += response?.cues.length ?? 0;
      } catch {
        // El núcleo local no respondió esta vez: se vuelve a pedir en la siguiente vuelta.
      }
      if (!cancelled) timer = window.setTimeout(poll, SUBTITLE_POLL_MS);
    };
    void poll();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      textTrack.mode = "disabled";
      if (textTrackRef.current === textTrack) textTrackRef.current = null;
    };
    // La URL y `retryNonce` deciden la `key` del `<video>`: cada cambio es un elemento nuevo.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamUrl, retryNonce, subtitleTrack, resumeReady]);

  useEffect(() => {
    subtitleLineRef.current = controlsVisible ? SUBTITLE_LINE_VISIBLE_CONTROLS : SUBTITLE_LINE_HIDDEN_CONTROLS;
    for (const cue of Array.from(textTrackRef.current?.cues ?? [])) {
      if (cue instanceof VTTCue) cue.line = subtitleLineRef.current;
    }
  }, [controlsVisible]);

  // Salto pendiente en el nivel 2 (ver `SEEK_SETTLE_MS`): mientras tanto, la barra y el
  // reloj enseñan a dónde se va, y los saltos relativos cuentan desde ahí.
  const [pendingSeek, setPendingSeek] = useState<number | null>(null);
  const pendingSeekTimerRef = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(pendingSeekTimerRef.current), []);
  const seekBase = pendingSeek ?? displayCurrentTime;
  const seekTo = useCallback(
    (targetSeconds: number) => {
      const clamped = Math.max(0, displayDuration ? Math.min(targetSeconds, displayDuration - 1) : targetSeconds);
      if (nativeSeekable && videoRef.current) {
        videoRef.current.currentTime = clamped;
        return;
      }
      setPendingSeek(clamped);
      window.clearTimeout(pendingSeekTimerRef.current);
      pendingSeekTimerRef.current = window.setTimeout(() => {
        setAppliedStart(Math.floor(clamped));
        setVideoCurrentTime(0);
        setPendingSeek(null);
      }, SEEK_SETTLE_MS);
    },
    [displayDuration, nativeSeekable],
  );

  const togglePlay = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    // En directo: pausar deja el canal grabándose; reanudar sigue desde la grabación,
    // donde se pausó.
    if (liveTimeshift && !timeshift.inBuffer) {
      if (video.paused) timeshift.resumeFromPause();
      else {
        video.pause();
        timeshift.notePaused();
      }
      return;
    }
    if (video.paused) video.play().catch(() => {});
    else video.pause();
  }, [liveTimeshift, timeshift]);

  const toggleMute = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
    setMuted(video.muted);
  }, []);

  const changeVolume = useCallback((delta: number) => {
    setVolume((v) => Math.round(Math.min(1, Math.max(0, v + delta)) * 100) / 100);
    const video = videoRef.current;
    if (delta > 0 && video?.muted) {
      video.muted = false;
      setMuted(false);
    }
  }, []);

  const togglePictureInPicture = useCallback(() => {
    const video = videoRef.current;
    if (!video || !document.pictureInPictureEnabled) return;
    if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
    else video.requestPictureInPicture().catch(() => {});
  }, []);

  const changeRate = useCallback((direction: 1 | -1) => {
    setPlaybackRate((rate) => {
      const index = PLAYBACK_RATES.indexOf(rate);
      return PLAYBACK_RATES[Math.max(0, Math.min(PLAYBACK_RATES.length - 1, (index === -1 ? 2 : index) + direction))] ?? 1;
    });
  }, []);

  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else containerRef.current?.requestFullscreen().catch(() => {});
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      // En un selector de pista, flechas y espacio son para elegir opción, no para
      // volumen/búsqueda/pausa; Escape sí sigue saliendo del reproductor.
      if (e.target instanceof HTMLSelectElement && e.key !== "Escape") {
        resetHideTimer();
        return;
      }
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
        case "p":
          togglePictureInPicture();
          break;
        case ">":
          if (kind !== "channel") changeRate(1);
          break;
        case "<":
          if (kind !== "channel") changeRate(-1);
          break;
        case "s":
          if (inIntro && introEnd != null) seekTo(introEnd);
          break;
        case "f":
          toggleFullscreen();
          break;
        case "+":
        case "=":
          changeVolume(VOLUME_STEP * 2);
          break;
        case "-":
          changeVolume(-VOLUME_STEP * 2);
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
          if (kind !== "channel") seekTo(seekBase - SEEK_STEP_SECONDS);
          else if (liveTimeshift) timeshift.seekBy(-SEEK_STEP_SECONDS);
          break;
        case "ArrowRight":
          if (kind !== "channel") seekTo(seekBase + SEEK_STEP_SECONDS);
          else if (liveTimeshift) timeshift.seekBy(SEEK_STEP_SECONDS);
          break;
        default:
          // En directo, las cifras son el número de canal (ver `typedNumber`).
          if (liveTimeshift && /^[0-9]$/.test(e.key)) {
            e.preventDefault();
            setNumberMessage(null);
            setTypedNumber((typed) => (typed + e.key).slice(0, 4));
            break;
          }
          // 0-9 salta directo a ese decil del vídeo (0%, 10%... 90%) — mismo atajo que
          // YouTube, y que el mando/teclado ya puede pulsar sin necesitar la barra.
          if (kind !== "channel" && displayDuration && /^[0-9]$/.test(e.key)) {
            e.preventDefault();
            seekTo((displayDuration * Number(e.key)) / 10);
          }
      }
      resetHideTimer();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    kind,
    goBack,
    togglePlay,
    toggleMute,
    toggleFullscreen,
    zapChannel,
    seekTo,
    seekBase,
    resetHideTimer,
    changeVolume,
    liveTimeshift,
    timeshift,
    togglePictureInPicture,
    changeRate,
    inIntro,
    introEnd,
  ]);

  useEffect(() => {
    const video = videoRef.current;
    if (video) video.volume = volume;
  }, [volume, streamPath, retryNonce, timeshift.bufferKey]);
  useEffect(() => {
    try {
      window.localStorage.setItem(VOLUME_STORAGE_KEY, String(volume));
    } catch {
      // Sin almacenamiento (modo privado del WebView): solo no se recuerda.
    }
  }, [volume]);

  // Velocidad (F14): solo en VOD. Cada búsqueda o cambio de pista monta otro `<video>`,
  // que empieza a 1×: se le vuelve a aplicar.
  useEffect(() => {
    const video = videoRef.current;
    if (video && kind !== "channel") video.playbackRate = playbackRate;
  }, [playbackRate, kind, streamPath, retryNonce]);

  // Imagen en imagen (F14): el estado lo lleva el navegador; aquí solo se sigue para el
  // botón.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const onEnter = () => setInPictureInPicture(true);
    const onLeave = () => setInPictureInPicture(false);
    video.addEventListener("enterpictureinpicture", onEnter);
    video.addEventListener("leavepictureinpicture", onLeave);
    return () => {
      video.removeEventListener("enterpictureinpicture", onEnter);
      video.removeEventListener("leavepictureinpicture", onLeave);
    };
  }, [streamPath, retryNonce, timeshift.bufferKey]);

  const canRenderVideo = streamUrl && resumeReady;

  return (
    <div
      ref={containerRef}
      data-vibe-player=""
      className={`relative flex h-full w-full items-center justify-center bg-black outline-none ${controlsVisible ? "" : "cursor-none"}`}
      onMouseMove={resetHideTimer}
      onWheel={(e) => {
        if (e.deltaY === 0) return;
        changeVolume(e.deltaY < 0 ? VOLUME_STEP : -VOLUME_STEP);
        resetHideTimer();
      }}
      tabIndex={-1}
    >
      <style>{subtitleCueCss(subtitleStyle, "[data-vibe-player]")}</style>
      {canRenderVideo ? (
        <video
          key={timeshift.bufferKey ?? `${streamPath}::${retryNonce}`}
          ref={videoRef}
          src={timeshift.inBuffer ? undefined : streamUrl}
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
            tierQuery.refetch();
          }}
          onTimeUpdate={(e) => setVideoCurrentTime(e.currentTarget.currentTime)}
          onPlaying={() => {
            if (firstFrameMsRef.current == null) firstFrameMsRef.current = Math.round(performance.now() - mountedAtRef.current);
            setBuffering(false);
            // Ya está reproduciendo de verdad tras el corte: los intentos se olvidan,
            // para no dejar un canal que en general va bien sin reintentos la próxima
            // vez que le toque un mal minuto.
            setReconnectAttempt(0);
          }}
          onDoubleClick={toggleFullscreen}
          onPlay={() => {
            setPlaying(true);
            resetHideTimer();
          }}
          onPause={() => {
            setPlaying(false);
            if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
            setControlsVisible(true);
          }}
          onWaiting={() => setBuffering(true)}
          onStalled={() => setBuffering(true)}
          onCanPlay={() => setBuffering(false)}
          onVolumeChange={(e) => setMuted(e.currentTarget.muted)}
          onError={handleVideoError}
        />
      ) : (
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-neutral-700 border-t-red-600" aria-label="Cargando" />
      )}

      {!errorMessage && reconnectAttempt > 0 && (
        <div className="absolute left-1/2 top-6 flex -translate-x-1/2 items-center gap-2 rounded-lg bg-black/80 px-4 py-2 text-sm text-neutral-200">
          <span className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-neutral-600 border-t-red-600" />
          Reconectando… (intento {reconnectAttempt}/{MAX_RECONNECT_ATTEMPTS})
        </div>
      )}

      {!errorMessage && reconnectAttempt === 0 && buffering && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="h-10 w-10 animate-spin rounded-full border-4 border-white/20 border-t-white/80" aria-label="Cargando" />
        </div>
      )}

      {sourceNotice && (
        <div className="absolute left-1/2 top-6 -translate-x-1/2 rounded-lg bg-black/80 px-4 py-2 text-sm text-neutral-200" role="status">
          {sourceNotice}
        </div>
      )}

      {resumeBanner != null && (
        <div className="absolute left-6 top-6 rounded-lg bg-black/70 px-4 py-2 text-sm text-neutral-200">
          Reanudando desde {formatTime(resumeBanner)}
        </div>
      )}

      {(typedNumber || numberMessage) && (
        <div className="absolute right-8 top-6 rounded-lg bg-black/80 px-5 py-3 text-right" aria-live="polite">
          {typedNumber ? (
            <p className="text-4xl font-semibold tabular-nums text-white">{typedNumber === "0" ? "Anterior" : typedNumber}</p>
          ) : (
            <p className="text-sm text-neutral-300">{numberMessage}</p>
          )}
        </div>
      )}

      {kind === "channel" && channelOverlayVisible && (
        <div className="absolute left-6 top-6 max-w-sm rounded-lg bg-black/80 px-4 py-3">
          <p className="font-medium text-white">{currentChannel?.display_name}</p>
          {channelEpgQuery.data?.[0]?.current ? (
            <>
              <p className="mt-0.5 truncate text-sm text-neutral-300">{channelEpgQuery.data[0].current.title}</p>
              {channelEpgQuery.data[0].next && (
                <p className="mt-0.5 truncate text-xs text-neutral-500">A continuación: {channelEpgQuery.data[0].next.title}</p>
              )}
            </>
          ) : (
            <p className="mt-0.5 text-sm text-neutral-500">Sin guía disponible</p>
          )}
        </div>
      )}

      {playingInMpv && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 bg-black px-10 text-center text-neutral-200">
          <p className="max-w-md text-lg">Se está reproduciendo en una ventana aparte (mpv).</p>
          <p className="max-w-md text-sm text-neutral-400">
            Este contenido usa un formato que el reproductor de la app no abre. Cierra esa ventana para terminar.
          </p>
          <button
            type="button"
            autoFocus
            onClick={goBack}
            className="rounded-lg bg-neutral-700 px-5 py-2.5 font-medium text-white outline-none hover:bg-neutral-600"
          >
            Volver
          </button>
        </div>
      )}

      {errorMessage && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 bg-black/80 px-10 text-center text-neutral-200">
          <p className="max-w-md">{errorMessage}</p>
          {needsMpv && (
            <button
              type="button"
              onClick={launchMpv}
              disabled={mpvLaunching}
              className="rounded-lg bg-red-600 px-5 py-2.5 font-medium text-white outline-none hover:bg-red-500 disabled:opacity-50"
            >
              {mpvLaunching ? "Abriendo…" : "Abrir con mpv (ventana externa)"}
            </button>
          )}
        </div>
      )}

      {inIntro && introEnd != null && (
        <button
          type="button"
          onClick={() => seekTo(introEnd)}
          className="absolute bottom-32 right-8 rounded-lg border border-white/40 bg-black/70 px-5 py-3 font-medium text-white outline-none hover:bg-black/90 focus-visible:ring-2 focus-visible:ring-white"
          title="Saltar intro (S)"
        >
          Saltar intro
        </button>
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
          {liveTimeshift && (
            <StatRow label="Modo" value={timeshift.inBuffer ? `grabación (−${formatTime(timeshift.behindLive(videoCurrentTime))})` : "directo"} />
          )}
          <StatRow label="Vídeo" value={tierQuery.data?.video_codec ?? "—"} />
          <StatRow label="Audio" value={tierQuery.data?.audio_codec ?? "—"} />
          <StatRow label="Contenedor" value={tierQuery.data?.container ?? "—"} />
          <StatRow label="Duración" value={displayDuration != null ? formatTime(displayDuration) : "—"} />
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

        {liveTimeshift && timeshift.inBuffer && timeshift.liveEdge != null && timeshift.windowStart != null ? (
          <input
            type="range"
            min={timeshift.windowStart}
            max={timeshift.liveEdge}
            step={1}
            value={Math.min(Math.max(videoCurrentTime, timeshift.windowStart), timeshift.liveEdge)}
            onChange={(e) => {
              if (videoRef.current) videoRef.current.currentTime = Number(e.target.value);
            }}
            className="w-full accent-red-600"
            aria-label="Posición en lo grabado del canal"
          />
        ) : null}

        {kind !== "channel" && displayDuration ? (
          <input
            type="range"
            min={0}
            max={Math.max(displayDuration, 1)}
            value={Math.min(seekBase, displayDuration)}
            onChange={(e) => seekTo(Number(e.target.value))}
            className="w-full accent-red-600"
            aria-label="Progreso"
          />
        ) : null}

        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-white">
          <button type="button" onClick={togglePlay} className="outline-none" aria-label={playing ? "Pausar" : "Reproducir"}>
            {playing ? <PauseIcon className="h-7 w-7" /> : <PlayIcon className="h-7 w-7" />}
          </button>

          {kind !== "channel" && (
            <div className="flex items-center gap-2 text-sm">
              <button
                type="button"
                onClick={() => seekTo(seekBase - SEEK_STEP_SECONDS)}
                className="rounded px-2 py-1 outline-none hover:bg-white/10"
                title="Retroceder 10 s (flecha izquierda)"
              >
                «10s
              </button>
              <button
                type="button"
                onClick={() => seekTo(seekBase + SEEK_STEP_SECONDS)}
                className="rounded px-2 py-1 outline-none hover:bg-white/10"
                title="Avanzar 10 s (flecha derecha)"
              >
                10s»
              </button>
            </div>
          )}

          {liveTimeshift && (
            <LiveTimeshiftControls
              inBuffer={timeshift.inBuffer}
              behind={timeshift.behindLive(videoCurrentTime)}
              onBack={() => timeshift.seekBy(-SEEK_STEP_SECONDS)}
              onForward={() => timeshift.seekBy(SEEK_STEP_SECONDS)}
              onGoLive={timeshift.goLive}
            />
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
              {formatTime(seekBase)} {displayDuration ? `/ ${formatTime(displayDuration)}` : ""}
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
            className="w-20 accent-red-600 xl:w-24"
            aria-label="Volumen"
          />

          {kind !== "channel" && (tierQuery.data?.audio_tracks.length ?? 0) > 1 && (
            <label className="flex items-center gap-1.5 text-xs text-neutral-400">
              <span className="hidden 2xl:inline">Audio</span>
              <select
                value={search.audioTrack ?? 0}
                onChange={(e) => setAudioTrack(Number(e.target.value))}
                className="rounded bg-black/60 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-white/60"
                aria-label="Pista de audio"
              >
                {tierQuery.data!.audio_tracks.map((track) => (
                  <option key={track.index} value={track.index}>
                    {audioTrackLabel(track)}
                  </option>
                ))}
              </select>
            </label>
          )}

          {kind !== "channel" && (tierQuery.data?.subtitle_tracks.length ?? 0) > 0 && (
            <label className="flex items-center gap-1.5 text-xs text-neutral-400">
              <span className="hidden 2xl:inline">Subtítulos</span>
              <select
                value={subtitleTrack ?? -1}
                onChange={(e) => chooseSubtitles(Number(e.target.value) < 0 ? null : Number(e.target.value))}
                className="rounded bg-black/60 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-white/60"
                aria-label="Subtítulos"
              >
                <option value={-1}>No</option>
                {tierQuery.data!.subtitle_tracks.map((track) => (
                  <option key={track.index} value={track.index}>
                    {track.language ? languageName(track.language) : `Pista ${track.index + 1}`}
                  </option>
                ))}
              </select>
            </label>
          )}

          {subtitleTrack != null && (
            <label className="flex items-center gap-1.5 text-xs text-neutral-400">
              <span className="hidden 2xl:inline">Tamaño</span>
              <select
                value={subtitleStyle.size}
                onChange={(e) => setSubtitleStyle({ ...subtitleStyle, size: e.target.value })}
                className="rounded bg-black/60 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-white/60"
                aria-label="Tamaño de los subtítulos"
                title="Más opciones en Ajustes → Subtítulos"
              >
                {SUBTITLE_SIZES.map((size) => (
                  <option key={size.value} value={size.value}>
                    {size.label}
                  </option>
                ))}
              </select>
            </label>
          )}

          {movieSources.length > 1 && (
            <label className="flex items-center gap-1.5 text-xs text-neutral-400">
              <span className="hidden 2xl:inline">Fuente</span>
              <select
                value={activeSourceIndex}
                onChange={(e) => switchSource(Number(e.target.value), false)}
                className="max-w-[9rem] truncate rounded bg-black/60 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-white/60 2xl:max-w-[12rem]"
                aria-label="Fuente"
              >
                {movieSources.map((source, index) => (
                  <option key={source.id} value={index}>
                    {sourceLabel(source, index)}
                  </option>
                ))}
              </select>
            </label>
          )}

          {kind !== "channel" && (
            <label className="flex items-center gap-1.5 text-xs text-neutral-400">
              <span className="hidden 2xl:inline">Velocidad</span>
              <select
                value={playbackRate}
                onChange={(e) => setPlaybackRate(Number(e.target.value))}
                className="rounded bg-black/60 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-white/60"
                aria-label="Velocidad de reproducción"
                title="Velocidad (< y >)"
              >
                {PLAYBACK_RATES.map((rate) => (
                  <option key={rate} value={rate}>
                    {rate === 1 ? "Normal" : `${String(rate).replace(".", ",")}×`}
                  </option>
                ))}
              </select>
            </label>
          )}

          {typeof document !== "undefined" && document.pictureInPictureEnabled && (
            <button
              type="button"
              onClick={togglePictureInPicture}
              aria-pressed={inPictureInPicture}
              className="flex items-center gap-1.5 rounded px-2 py-1 text-xs text-neutral-300 outline-none hover:bg-white/10 focus-visible:ring-2 focus-visible:ring-white/60"
              title="Imagen en imagen (P)"
              aria-label={inPictureInPicture ? "Salir de imagen en imagen" : "Imagen en imagen"}
            >
              <PictureInPictureIcon className="h-5 w-5" />
              <span className="hidden 2xl:inline">{inPictureInPicture ? "Salir de imagen en imagen" : "Imagen en imagen"}</span>
            </button>
          )}

          <button
            type="button"
            onClick={goBack}
            className="ml-auto flex items-center gap-1.5 text-sm text-neutral-300 outline-none hover:text-white"
            aria-label="Volver (Esc)"
            title="Volver (Esc)"
          >
            <BackIcon className="h-5 w-5" />
            <span className="hidden xl:inline">Volver (Esc)</span>
          </button>
          <button type="button" onClick={toggleFullscreen} className="outline-none" aria-label="Pantalla completa">
            <FullscreenIcon className="h-5 w-5" />
          </button>
        </div>
      </div>
    </div>
  );
}

/** Rebobinar/avanzar y estado respecto al directo (F13). */
function LiveTimeshiftControls({
  inBuffer,
  behind,
  onBack,
  onForward,
  onGoLive,
}: {
  inBuffer: boolean;
  behind: number;
  onBack: () => void;
  onForward: () => void;
  onGoLive: () => void;
}) {
  const atLive = !inBuffer || behind < 8;
  return (
    <div className="flex items-center gap-2 text-sm">
      <button
        type="button"
        onClick={onBack}
        className="rounded px-2 py-1 outline-none hover:bg-white/10 focus-visible:ring-2 focus-visible:ring-white/60"
        title="Retroceder 10 s (flecha izquierda)"
      >
        «10s
      </button>
      <button
        type="button"
        onClick={onForward}
        disabled={atLive}
        className="rounded px-2 py-1 outline-none hover:bg-white/10 focus-visible:ring-2 focus-visible:ring-white/60 disabled:opacity-40"
        title="Avanzar 10 s (flecha derecha)"
      >
        10s»
      </button>
      {atLive ? (
        <span className="flex items-center gap-1.5 rounded bg-red-600 px-2 py-0.5 text-xs font-semibold uppercase tracking-wide text-white">
          <span className="h-1.5 w-1.5 rounded-full bg-white" />
          En directo
        </span>
      ) : (
        <>
          <span className="w-16 tabular-nums text-neutral-300">−{formatTime(behind)}</span>
          <button
            type="button"
            onClick={onGoLive}
            className="rounded bg-white/10 px-2 py-1 text-xs font-medium outline-none hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white/60"
          >
            Ir al directo
          </button>
        </>
      )}
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

/// Nombre de idioma legible para los códigos ISO-639-2 más habituales en paneles Xtream
/// (español/inglés casi siempre, a veces algún otro) — sin librería de localización solo
/// para esto; un código sin traducir en mayúsculas sigue siendo mejor que nada.
const LANGUAGE_NAMES: Record<string, string> = {
  spa: "Español",
  eng: "Inglés",
  lat: "Latino",
  fre: "Francés",
  fra: "Francés",
  ger: "Alemán",
  deu: "Alemán",
  ita: "Italiano",
  por: "Portugués",
  und: "Sin idioma",
};

function languageName(code: string): string {
  return LANGUAGE_NAMES[code.toLowerCase()] ?? code.toUpperCase();
}

function audioTrackLabel(track: { index: number; language: string | null; channels: number | null }): string {
  const language = track.language ? languageName(track.language) : `Pista ${track.index + 1}`;
  return track.channels === 6 ? `${language} 5.1` : language;
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
