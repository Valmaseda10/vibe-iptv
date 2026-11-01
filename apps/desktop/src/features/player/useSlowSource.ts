import { useEffect, useRef, type RefObject } from "react";
import { LONG_STALL_MS, SLOW_START_MS, stallSummary, stallsSaySlow, type Stall } from "./movieSources";

/** Tras un salto, esperar a que cargue el punto nuevo no es un parón. */
const AFTER_SEEK_GRACE_MS = 3_000;

export type SlowReason = "slow_start" | "stalls";

/**
 * Vigila si la fuente de una película o un episodio va lenta en el `<video>` del PC (F17):
 * tarda más de `SLOW_START_MS` en empezar a verse o se para demasiado (`stallsSaySlow`).
 * Avisa una vez por vídeo (`videoKey`); el reproductor decide si cambia de fuente. La regla
 * es la misma del iPhone, que cuenta sus parones desde VLC (`NativePlayerScreen`).
 */
export function useSlowSource(
  videoRef: RefObject<HTMLVideoElement | null>,
  active: boolean,
  videoKey: string,
  onSlow: (reason: SlowReason, summary: { stalls: number; waitedMs: number }) => void,
) {
  const onSlowRef = useRef(onSlow);
  onSlowRef.current = onSlow;

  useEffect(() => {
    const video = videoRef.current;
    if (!active || !video) return;
    let fired = false;
    let played = false;
    let lastSeekAt = -Infinity;
    let stallStartedAt: number | null = null;
    const stalls: Stall[] = [];
    let longTimer: number | undefined;

    const fire = (reason: SlowReason, summary: { stalls: number; waitedMs: number }) => {
      if (fired) return;
      fired = true;
      onSlowRef.current(reason, summary);
    };
    const evaluate = () => {
      const now = performance.now();
      if (stallsSaySlow(stalls, now)) fire("stalls", stallSummary(stalls, now));
    };

    const startTimer = window.setTimeout(() => {
      // En pausa antes de empezar, la espera es del usuario.
      if (!played && !video.paused) fire("slow_start", { stalls: 0, waitedMs: SLOW_START_MS });
    }, SLOW_START_MS);

    const onPlaying = () => {
      played = true;
      window.clearTimeout(startTimer);
      window.clearTimeout(longTimer);
      if (stallStartedAt !== null) {
        const now = performance.now();
        // Si ya se apuntó como parón largo, no se cuenta dos veces.
        const ms = now - stallStartedAt;
        if (ms < LONG_STALL_MS) stalls.push({ at: now, ms });
        stallStartedAt = null;
        evaluate();
      }
    };
    const onSeeking = () => {
      lastSeekAt = performance.now();
    };
    const onWaiting = () => {
      const now = performance.now();
      if (!played || video.paused || stallStartedAt !== null || now - lastSeekAt < AFTER_SEEK_GRACE_MS) return;
      stallStartedAt = now;
      longTimer = window.setTimeout(() => {
        stalls.push({ at: performance.now(), ms: LONG_STALL_MS });
        evaluate();
      }, LONG_STALL_MS);
    };

    video.addEventListener("playing", onPlaying);
    video.addEventListener("seeking", onSeeking);
    video.addEventListener("waiting", onWaiting);
    return () => {
      window.clearTimeout(startTimer);
      window.clearTimeout(longTimer);
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("seeking", onSeeking);
      video.removeEventListener("waiting", onWaiting);
    };
  }, [videoRef, active, videoKey]);
}
