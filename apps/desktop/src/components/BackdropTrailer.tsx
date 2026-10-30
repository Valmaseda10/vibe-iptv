import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IS_IOS } from "../lib/platform";
import { MuteIcon, VolumeIcon } from "./icons";

const YOUTUBE_ORIGIN = "https://www.youtube-nocookie.com";

type Props = {
  youtubeKey: string;
  /** También en el PC (la portada). Las fichas lo tienen solo en iPhone/iPad. */
  desktop?: boolean;
  /** `false` lo pausa y vuelve al fondo fijo: fuera de la vista, ventana oculta, algo sonando. */
  active?: boolean;
  /** Espera antes de cargar nada: si solo se está pasando, no se descarga el tráiler. */
  delayMs?: number;
  /** Botón para activar el sonido, abajo a la derecha. */
  soundToggle?: boolean;
};

/** Tráiler de fondo, en silencio y sin controles, como Netflix (F15). En iOS solo con
 * Wi‑Fi (`network_is_metered`), y solo se ve cuando YouTube confirma que está
 * reproduciendo: si el embed falla o el sistema no deja reproducir solo, el fondo se
 * queda como estaba, sin cajas de error. Al acabar, vuelve el fondo. No es un stream
 * del proveedor, así que no pasa por `StreamGuard` (R1). */
export function BackdropTrailer({ youtubeKey, desktop = false, active = true, delayMs = 1500, soundToggle = false }: Props) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const loadedRef = useRef(false);
  const [allowed, setAllowed] = useState(false);
  const [visible, setVisible] = useState(false);
  const [muted, setMuted] = useState(true);
  const platformOk = IS_IOS || desktop;

  useEffect(() => {
    loadedRef.current = false;
    setAllowed(false);
    setVisible(false);
    setMuted(true);
  }, [youtubeKey]);

  useEffect(() => {
    if (!platformOk || !active || allowed) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      invoke<boolean>("network_is_metered")
        .then((metered) => {
          if (!cancelled && !metered) setAllowed(true);
        })
        .catch(() => {});
    }, delayMs);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [youtubeKey, platformOk, active, allowed, delayMs]);

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

  useEffect(() => {
    // Antes de cargar, el `onLoad` ya lo tiene en cuenta.
    if (allowed && loadedRef.current) command(frameRef.current, active ? "playVideo" : "pauseVideo");
  }, [allowed, active]);

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
  const shown = visible && active;

  return (
    <>
      <iframe
        ref={frameRef}
        src={`${YOUTUBE_ORIGIN}/embed/${youtubeKey}?${params.toString()}`}
        title=""
        aria-hidden
        tabIndex={-1}
        allow="autoplay; encrypted-media"
        // Para que YouTube mande el estado del reproductor por `postMessage`.
        onLoad={() => {
          loadedRef.current = true;
          frameRef.current?.contentWindow?.postMessage(JSON.stringify({ event: "listening", id: 1, channel: "widget" }), YOUTUBE_ORIGIN);
          if (!activeRef.current) command(frameRef.current, "pauseVideo");
        }}
        // Cubre todo el fondo (16:9 recortado) y un poco más, para que no asomen los bordes
        // ni la barra de título del embed.
        className={`pointer-events-none absolute left-1/2 top-1/2 aspect-video min-h-full min-w-full -translate-x-1/2 -translate-y-1/2 scale-[1.35] border-0 transition-opacity duration-1000 ${shown ? "opacity-100" : "opacity-0"}`}
        style={{ width: "max(100%, calc(var(--trailer-h, 60vh) * 16 / 9))" }}
      />
      {soundToggle && shown && (
        <button
          type="button"
          onClick={() => {
            command(frameRef.current, muted ? "unMute" : "mute");
            setMuted(!muted);
          }}
          aria-label={muted ? "Activar el sonido del tráiler" : "Silenciar el tráiler"}
          aria-pressed={!muted}
          className="preview-in absolute bottom-8 right-4 z-10 rounded-full border border-white/60 bg-black/30 p-2.5 text-white outline-none backdrop-blur transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-red-600 sm:bottom-10 sm:right-10 ios:glass-clear ios:border-0"
        >
          {muted ? <MuteIcon className="h-5 w-5" /> : <VolumeIcon className="h-5 w-5" />}
        </button>
      )}
    </>
  );
}

/** Orden al reproductor del embed por el API de iframes de YouTube. */
function command(frame: HTMLIFrameElement | null, func: "playVideo" | "pauseVideo" | "mute" | "unMute") {
  frame?.contentWindow?.postMessage(JSON.stringify({ event: "command", func, args: [] }), YOUTUBE_ORIGIN);
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
