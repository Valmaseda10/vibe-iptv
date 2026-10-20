import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IS_IOS } from "../lib/platform";

const YOUTUBE_ORIGIN = "https://www.youtube-nocookie.com";

/** Tráiler de fondo de una ficha, en silencio y sin controles, como Netflix (F15). Solo
 * en iOS y con Wi‑Fi (`network_is_metered`) y solo se ve cuando YouTube confirma que está
 * reproduciendo: si el embed falla o el sistema no deja reproducir solo, el fondo se
 * queda como estaba, sin cajas de error. Al acabar, vuelve el fondo. No es un stream
 * del proveedor, así que no pasa por `StreamGuard` (R1). */
export function BackdropTrailer({ youtubeKey }: { youtubeKey: string }) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const [allowed, setAllowed] = useState(false);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setAllowed(false);
    setVisible(false);
    // Solo en iPhone/iPad (lo pidió el usuario para la app móvil; el PC no cambia).
    if (!IS_IOS) return;
    // Un instante de espera: si solo se está pasando por la ficha, no se carga nada.
    const timer = window.setTimeout(() => {
      invoke<boolean>("network_is_metered")
        .then((metered) => {
          if (!cancelled && !metered) setAllowed(true);
        })
        .catch(() => {});
    }, 1500);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [youtubeKey]);

  useEffect(() => {
    if (!allowed) return;
    let revealTimer: number | undefined;
    function onMessage(event: MessageEvent) {
      if (event.origin !== YOUTUBE_ORIGIN || event.source !== frameRef.current?.contentWindow) return;
      let data: unknown = event.data;
      if (typeof data === "string") {
        try {
          data = JSON.parse(data);
        } catch {
          return;
        }
      }
      const state = playerState(data);
      if (state === 1) {
        // El título de YouTube asoma al principio: se revela cuando ya se ha ido.
        window.clearTimeout(revealTimer);
        revealTimer = window.setTimeout(() => setVisible(true), 1500);
      } else if (state === 0) {
        window.clearTimeout(revealTimer);
        setVisible(false);
      }
    }
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("message", onMessage);
      window.clearTimeout(revealTimer);
    };
  }, [allowed]);

  if (!allowed) return null;

  const params = new URLSearchParams({
    autoplay: "1",
    mute: "1",
    controls: "0",
    playsinline: "1",
    rel: "0",
    iv_load_policy: "3",
    disablekb: "1",
    fs: "0",
    enablejsapi: "1",
  });

  return (
    <iframe
      ref={frameRef}
      src={`${YOUTUBE_ORIGIN}/embed/${youtubeKey}?${params.toString()}`}
      title=""
      aria-hidden
      tabIndex={-1}
      allow="autoplay; encrypted-media"
      // Para que YouTube mande el estado del reproductor por `postMessage`.
      onLoad={() => frameRef.current?.contentWindow?.postMessage(JSON.stringify({ event: "listening", id: 1, channel: "widget" }), YOUTUBE_ORIGIN)}
      // Cubre todo el fondo (16:9 recortado) y un poco más, para que no asomen los bordes
      // ni la barra de título del embed.
      className={`pointer-events-none absolute left-1/2 top-1/2 aspect-video min-h-full min-w-full -translate-x-1/2 -translate-y-1/2 scale-[1.35] border-0 transition-opacity duration-1000 ${visible ? "opacity-100" : "opacity-0"}`}
      style={{ width: "max(100%, calc(var(--trailer-h, 60vh) * 16 / 9))" }}
    />
  );
}

/** Estado del reproductor en los mensajes del API de iframes (1 reproduciendo, 0 terminado). */
function playerState(data: unknown): number | null {
  if (typeof data !== "object" || data === null) return null;
  const message = data as { event?: unknown; info?: unknown };
  if (message.event === "onStateChange" && typeof message.info === "number") return message.info;
  if (message.event === "infoDelivery" && typeof message.info === "object" && message.info !== null) {
    const info = message.info as { playerState?: unknown };
    if (typeof info.playerState === "number") return info.playerState;
  }
  return null;
}
