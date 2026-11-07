import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type Hls from "hls.js";

/** Cada cuánto avisa el reproductor de que sigue en el canal: el núcleo cierra la
 * grabación (y libera la conexión con el proveedor) tras 20 s sin noticias. */
const KEEPALIVE_MS = 5_000;
/** Al reanudar se vuelve un poco antes de donde se pausó: mejor repetir un segundo que
 * saltárselo (el directo va algo por delante del último trozo grabado). */
const RESUME_MARGIN_SECONDS = 2;

/** Parón con vídeo de sobra guardado: hls.js empuja la reproducción unas décimas antes
 * (a los 0,5 s, no a los 2 s) y lo intenta más veces antes de dar la grabación por perdida
 * (darla por perdida es volver a conectar con el proveedor). La causa de los que se
 * medían (huecos de audio cuando el proveedor pierde un trozo) se arregla en el núcleo:
 * ver `LIVE_AUDIO` en `media::ffmpeg`. */
const STALL_RECOVERY = { detectStallWithCurrentTimeMs: 500, highBufferWatchdogPeriod: 0.5, nudgeMaxRetry: 10 } as const;
/** Errores del decodificador que se arreglan sin salir de la grabación, por minuto. */
const MAX_RECOVERIES = 3;
const RECOVERY_WINDOW_MS = 60_000;

/** A dónde entrar en la grabación: el segundo exacto de la sesión y, por si la ventana ya
 * se deslizó, cuántos segundos por detrás del directo. */
type Entry = { at: number; behind: number; fromEdge?: boolean };

type Options = {
  /** Directo de un canal (no catch-up, no VOD): solo ahí hay grabación. */
  enabled: boolean;
  channelId: number;
  base: string | undefined;
  videoRef: RefObject<HTMLVideoElement | null>;
};

/** Pausar y rebobinar el directo (F13). El núcleo graba el canal mientras se ve (ver
 * `server::timeshift`); el reproductor lo ve en directo por la salida rápida de siempre
 * y, al pausar o rebobinar, pasa a la grabación con hls.js.
 *
 * Al entrar en la grabación se va al segundo exacto del directo mientras la lista empieza
 * en su primer trozo: ahí la línea de tiempo de hls.js es la de la sesión, la misma del
 * directo (las dos salidas de ffmpeg empiezan en cero). Cuando la ventana de 30 min ya se
 * ha deslizado, hls.js numera desde otro punto y solo queda contar "N segundos por
 * detrás del directo". No se usa siempre esto último: depende de que el canal llegue a
 * tiempo real (con el proveedor de pruebas, 10 min de pausa daban 9 s de desfase). */
export function useLiveTimeshift({ enabled, channelId, base, videoRef }: Options) {
  /** `null` = en directo. */
  const [entry, setEntry] = useState<Entry | null>(null);
  const [entryNonce, setEntryNonce] = useState(0);
  const [liveEdge, setLiveEdge] = useState<number | null>(null);
  const [windowStart, setWindowStart] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  const hlsRef = useRef<Hls | null>(null);
  const pausedAtRef = useRef<{ wall: number; media: number } | null>(null);

  // Otro canal: se empieza en directo.
  useEffect(() => {
    setEntry(null);
    setFailed(false);
    setLiveEdge(null);
    setWindowStart(null);
    pausedAtRef.current = null;
  }, [channelId]);

  // Señales de vida mientras se está en el canal (también en pausa, que no lee nada), y
  // al irse se cierra la grabación para liberar ya la conexión con el proveedor.
  useEffect(() => {
    if (!enabled || !base) return;
    const ping = () => fetch(`${base}/api/timeshift/${channelId}/keepalive`, { method: "POST" }).catch(() => {});
    const timer = window.setInterval(ping, KEEPALIVE_MS);
    return () => {
      window.clearInterval(timer);
      fetch(`${base}/api/timeshift/${channelId}`, { method: "DELETE" }).catch(() => {});
    };
  }, [enabled, base, channelId]);

  useEffect(() => {
    if (!enabled || entry == null || !base) return;
    const video = videoRef.current;
    if (!video) return;
    let cancelled = false;
    let started = false;
    void import("hls.js").then(({ default: HlsClass }) => {
      if (cancelled) return;
      const hls = new HlsClass({ autoStartLoad: false, backBufferLength: 90, maxBufferLength: 30, liveDurationInfinity: true, ...STALL_RECOVERY });
      const recoveries: number[] = [];
      hlsRef.current = hls;
      hls.on(HlsClass.Events.LEVEL_LOADED, (_event, data) => {
        setLiveEdge(data.details.edge);
        setWindowStart(data.details.fragments[0]?.start ?? 0);
        if (started) return;
        started = true;
        const first = data.details.fragments[0];
        const aligned = first?.sn === 0;
        const start = entry.fromEdge
          ? Math.max(first?.start ?? 0, data.details.edge - entry.behind)
          : Math.min(data.details.edge, Math.max(first?.start ?? 0, aligned ? entry.at : data.details.edge - entry.behind));
        hls.startLoad(start);
        video.play().catch(() => {});
      });
      hls.on(HlsClass.Events.ERROR, (_event, data) => {
        if (!data.fatal) return;
        // Un fotograma que el decodificador no traga (el proveedor perdió un trozo) se
        // arregla aquí mismo, sin volver a conectar con el proveedor. Si se repite mucho,
        // la grabación no sirve: se vuelve al directo, que sabe reconectar y explicarlo.
        const now = performance.now();
        while (now - (recoveries[0] ?? now) > RECOVERY_WINDOW_MS) recoveries.shift();
        if (data.type === HlsClass.ErrorTypes.MEDIA_ERROR && recoveries.length < MAX_RECOVERIES) {
          recoveries.push(now);
          hls.recoverMediaError();
          video.play().catch(() => {});
          return;
        }
        setFailed(true);
      });
      hls.loadSource(`${base}/stream/timeshift/channel/${channelId}/live.m3u8`);
      hls.attachMedia(video);
    });
    return () => {
      cancelled = true;
      hlsRef.current?.destroy();
      hlsRef.current = null;
    };
    // `entryNonce`: volver a entrar con el mismo retraso también reinicia.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, entry, entryNonce, base, channelId]);

  const enter = useCallback((next: Entry) => {
    setFailed(false);
    setEntry({ at: Math.max(0, next.at), behind: Math.max(0, next.behind) });
    setEntryNonce((n) => n + 1);
  }, []);

  /** Pasar a la grabación `behind` segundos por detrás de lo último grabado, sea cual sea el
   * punto en el que se estaba (tras un parón del directo: ver `useLiveCushion`). */
  const enterNearLive = useCallback((behind: number) => {
    pausedAtRef.current = null;
    setFailed(false);
    setEntry({ at: 0, behind, fromEdge: true });
    setEntryNonce((n) => n + 1);
  }, []);

  /** Pausa en directo: se apunta dónde y cuándo, para reanudar desde ese punto. */
  const notePaused = useCallback(() => {
    pausedAtRef.current = { wall: performance.now(), media: videoRef.current?.currentTime ?? 0 };
  }, [videoRef]);

  /** Reanudar tras una pausa en directo: se pasa a la grabación donde se pausó. */
  const resumeFromPause = useCallback(() => {
    const paused = pausedAtRef.current;
    pausedAtRef.current = null;
    const pausedFor = paused == null ? 0 : (performance.now() - paused.wall) / 1000;
    const at = paused?.media ?? videoRef.current?.currentTime ?? 0;
    enter({ at: at - RESUME_MARGIN_SECONDS, behind: pausedFor + RESUME_MARGIN_SECONDS });
  }, [enter, videoRef]);

  /** Mover la posición `delta` segundos (negativo = atrás), en directo o en la grabación. */
  const seekBy = useCallback(
    (delta: number) => {
      const video = videoRef.current;
      if (entry == null) {
        if (delta < 0 && video) enter({ at: video.currentTime + delta, behind: -delta });
        return;
      }
      if (!video || liveEdge == null) return;
      const lowest = windowStart ?? 0;
      video.currentTime = Math.min(liveEdge, Math.max(lowest, video.currentTime + delta));
    },
    [entry, enter, liveEdge, windowStart, videoRef],
  );

  /** Volver al directo dentro de la grabación (unos segundos por detrás del borde). */
  const goLive = useCallback(() => {
    const video = videoRef.current;
    const target = hlsRef.current?.liveSyncPosition ?? liveEdge;
    if (video && target != null) video.currentTime = target;
  }, [liveEdge, videoRef]);

  /** Cuánto se va por detrás del directo (solo en la grabación). */
  const behindLive = (currentTime: number) => (entry == null || liveEdge == null ? 0 : Math.max(0, liveEdge - currentTime));

  return {
    inBuffer: entry != null,
    /** Cambia cada vez que se entra en la grabación: sirve de `key` para el `<video>`. */
    bufferKey: entry == null ? null : `buffer-${entryNonce}`,
    failed,
    liveEdge,
    windowStart,
    notePaused,
    /** En pausa en directo (todavía sin pasar a la grabación). */
    isPausedLive: () => pausedAtRef.current != null,
    resumeFromPause,
    enterNearLive,
    seekBy,
    goLive,
    behindLive,
    leaveBuffer: () => setEntry(null),
  };
}
