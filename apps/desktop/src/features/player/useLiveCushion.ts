import { useEffect, useRef, type RefObject } from "react";

/** Un parón que dura más de esto pasa a la grabación; uno de un instante, solo si se repite. */
const SWITCH_AFTER_MS = 1000;
/** Se entra en la grabación esto por detrás de lo último grabado. La grabación avanza a
 * trozos de 2 s (o lo que dure un GOP del canal) y hls.js se entera de cada uno al volver a
 * pedir la lista: con 6 s quedaba poco margen real y se seguía parando; con unos 16 s, ni
 * un parón en dos minutos (medido). Hasta `LIVE_BADGE_MAX_BEHIND_S` se sigue diciendo
 * «En directo». */
export const LIVE_CUSHION_BEHIND_S = 12;
/** Retraso con el que la pantalla sigue diciendo «En directo» en vez de «−0:12». */
export const LIVE_BADGE_MAX_BEHIND_S = 15;

/** Por debajo de este margen en la grabación se reproduce un poco más despacio... */
const SLOW_BELOW_S = 6;
/** ...hasta recuperar este. */
const NORMAL_ABOVE_S = 9;
/** 5 % más lento: con el tono conservado (lo hace el navegador), no se nota. */
const SLOW_RATE = 0.95;

/**
 * Margen de la grabación (2026-11-07). Un canal que llega algo más despacio que el tiempo
 * real (proveedor saturado, VPN) se come poco a poco los segundos de margen y, al acabarse,
 * se para cada pocos segundos. VLC (iPhone y iPad) lo evita ajustando su reloj al del
 * canal; esto hace lo mismo: con poco margen, al 95 % hasta recuperarlo. Medido con el
 * proveedor de pruebas, que llega un 7 % lento: sin esto, parones de 2 s a partir del
 * minuto 3.
 */
export function useRecordingPace(videoRef: RefObject<HTMLVideoElement | null>, active: boolean, videoKey: string | null) {
  useEffect(() => {
    const video = videoRef.current;
    if (!active || !video) return;
    const timer = window.setInterval(() => {
      if (video.paused || !video.buffered.length) return;
      const ahead = video.buffered.end(video.buffered.length - 1) - video.currentTime;
      if (ahead < SLOW_BELOW_S && video.playbackRate !== SLOW_RATE) video.playbackRate = SLOW_RATE;
      else if (ahead > NORMAL_ABOVE_S && video.playbackRate !== 1) video.playbackRate = 1;
    }, 500);
    return () => {
      window.clearInterval(timer);
      video.playbackRate = 1;
    };
  }, [videoRef, active, videoKey]);
}

/** Lo que se cuenta de cada parón al núcleo, para el diagnóstico. */
export type StallReport = { aheadS: number; currentTime: number };

type Handlers = {
  /** Empieza el parón: apuntar dónde (como una pausa). */
  onStall: (report: StallReport) => void;
  /** Pasar a la grabación, `LIVE_CUSHION_BEHIND_S` por detrás de lo último grabado. */
  onSwitch: () => void;
};

/**
 * Parones del directo en el PC (2026-11-01). El directo se ve por la salida rápida (zapping
 * al instante), pero el `<video>` no guarda más de ~2,5 s por delante y cualquier bache del
 * proveedor era un parón: con el proveedor de pruebas con baches de 1,5 s cada 10 s, 3 por
 * minuto, y ni esperando más tras cada uno se conseguía margen (medido).
 *
 * Al primer parón de más de un segundo se pasa a la grabación del núcleo (la de pausar y
 * rebobinar, con hls.js), unos segundos por detrás de lo último grabado: a partir de ahí
 * los baches del proveedor solo frenan la grabación, no la imagen. Es lo que hace el iPhone
 * (VLC lee de la grabación). Medido con baches de 1,5 s cada 10 s: de 3 parones por minuto
 * a solo el primero en dos minutos.
 */
export function useLiveCushion(videoRef: RefObject<HTMLVideoElement | null>, active: boolean, videoKey: string, handlers: Handlers) {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  useEffect(() => {
    const video = videoRef.current;
    if (!active || !video) return;
    let timer: number | undefined;
    let stalls = 0;
    const ahead = () => (video.buffered.length ? video.buffered.end(video.buffered.length - 1) - video.currentTime : 0);

    const onWaiting = () => {
      if (timer !== undefined || video.paused || video.currentTime < 1) return;
      handlersRef.current.onStall({ aheadS: Number(ahead().toFixed(2)), currentTime: video.currentTime });
      stalls++;
      // El segundo parón en el mismo canal ya no es un instante suelto: a la grabación.
      if (stalls >= 2) {
        handlersRef.current.onSwitch();
        return;
      }
      const stalledAt = video.currentTime;
      timer = window.setTimeout(() => {
        timer = undefined;
        // Si entretanto el usuario pausó, se respeta su pausa (y reanudará él). Si ya siguió
        // solo, era un instante: se deja.
        if (!video.paused && video.currentTime - stalledAt < 0.5) handlersRef.current.onSwitch();
      }, SWITCH_AFTER_MS);
    };

    video.addEventListener("waiting", onWaiting);
    return () => {
      window.clearTimeout(timer);
      video.removeEventListener("waiting", onWaiting);
    };
  }, [videoRef, active, videoKey]);
}
