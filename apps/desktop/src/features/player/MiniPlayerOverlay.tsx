import type { MouseEvent } from "react";
import { CloseIcon, ExpandIcon, MuteIcon, PauseIcon, PlayIcon, VolumeIcon } from "../../components/icons";

type MiniPlayerOverlayProps = {
  title: string | undefined;
  playing: boolean;
  muted: boolean;
  buffering: boolean;
  error: string | null;
  onTogglePlay: () => void;
  onToggleMute: () => void;
  onExpand: () => void;
  onClose: () => void;
};

/** Controles de la ventana pequeña del reproductor: aparecen al pasar el ratón (y con el
 * foco del teclado). Pinchar en el vídeo lo devuelve a pantalla completa. */
export function MiniPlayerOverlay({ title, playing, muted, buffering, error, onTogglePlay, onToggleMute, onExpand, onClose }: MiniPlayerOverlayProps) {
  const stop = (action: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    action();
  };
  const button =
    "flex h-9 w-9 items-center justify-center rounded-full bg-black/50 text-white outline-none hover:bg-black/80 focus-visible:ring-2 focus-visible:ring-white";
  return (
    <div
      className="group/mini absolute inset-0 cursor-pointer"
      onClick={onExpand}
      title="Volver a pantalla completa"
    >
      {buffering && !error && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="h-7 w-7 animate-spin rounded-full border-[3px] border-white/20 border-t-white/80" aria-label="Cargando" />
        </div>
      )}
      {error && <p className="absolute inset-0 flex items-center justify-center bg-black/80 px-4 text-center text-xs text-neutral-200">{error}</p>}
      <div className="absolute inset-0 flex flex-col justify-between bg-gradient-to-b from-black/60 via-transparent to-black/70 p-2 opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover/mini:opacity-100">
        <div className="flex items-start justify-between gap-2">
          <button type="button" onClick={stop(onExpand)} className={button} aria-label="Pantalla completa" title="Pantalla completa">
            <ExpandIcon className="h-4 w-4" />
          </button>
          <button type="button" onClick={stop(onClose)} className={button} aria-label="Cerrar el reproductor" title="Cerrar">
            <CloseIcon className="h-4 w-4" />
          </button>
        </div>
        <div className="flex justify-center">
          <button type="button" onClick={stop(onTogglePlay)} className={`${button} h-11 w-11`} aria-label={playing ? "Pausar" : "Reproducir"}>
            {playing ? <PauseIcon className="h-6 w-6" /> : <PlayIcon className="h-6 w-6" />}
          </button>
        </div>
        <div className="flex items-center gap-2">
          <button type="button" onClick={stop(onToggleMute)} className={button} aria-label={muted ? "Quitar silencio" : "Silenciar"}>
            {muted ? <MuteIcon className="h-4 w-4" /> : <VolumeIcon className="h-4 w-4" />}
          </button>
          {title && <p className="min-w-0 flex-1 truncate text-xs font-medium text-white drop-shadow">{title}</p>}
        </div>
      </div>
    </div>
  );
}
