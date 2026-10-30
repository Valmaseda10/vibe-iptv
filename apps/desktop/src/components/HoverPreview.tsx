import { useEffect } from "react";
import { createPortal } from "react-dom";
import { imagePath, useApiBase } from "../lib/api-client";
import { useFavorites } from "../lib/favorites";
import { HeartIcon, PlayIcon, ChevronIcon } from "./icons";

/** Lo que enseña la tarjeta ampliada de un título. */
export type PreviewInfo = {
  kind: "movie" | "series";
  id: number;
  year?: number | null;
  rating?: number | null;
  genres?: string[];
  backdropPath?: string | null;
  /** Película: reproducirla directamente. Sin esto, ▶ abre la ficha. */
  onPlay?: () => void;
};

type HoverPreviewProps = {
  /** Dónde está la tarjeta en pantalla. */
  rect: DOMRect;
  title: string;
  posterPath: string | null;
  info: PreviewInfo;
  onOpen: () => void;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  onClose: () => void;
};

const WIDTH = 340;

/** Tarjeta ampliada al dejar el ratón sobre una portada (PC), como en Netflix: el fondo del
 * título, reproducir, «Mi lista», más información, año, nota y géneros. Flota encima de la
 * fila (en `body`, para que la fila no la recorte) y se va al salir, al desplazar o con Esc. */
export function HoverPreview({ rect, title, posterPath, info, onOpen, onPointerEnter, onPointerLeave, onClose }: HoverPreviewProps) {
  const { data: base } = useApiBase();
  const { isFavorite, toggleFavorite } = useFavorites();
  const inList = isFavorite(info.kind, info.id);
  const image = imagePath(base, info.backdropPath ?? posterPath, info.backdropPath ? "w500" : "w342");

  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("scroll", close, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
    };
  }, [onClose]);

  const height = (WIDTH * 9) / 16 + 118;
  const left = Math.min(Math.max(8, rect.left + rect.width / 2 - WIDTH / 2), window.innerWidth - WIDTH - 8);
  const top = Math.min(Math.max(8, rect.top + rect.height / 2 - height / 2), window.innerHeight - height - 8);
  const meta = [info.year ? String(info.year) : null, info.rating ? `★ ${info.rating.toFixed(1).replace(".", ",")}` : null, info.kind === "series" ? "Serie" : "Película"].filter(Boolean);
  const round = "flex h-9 w-9 items-center justify-center rounded-full outline-none transition focus-visible:ring-2 focus-visible:ring-white";

  return createPortal(
    <div
      className="preview-in fixed z-[70] overflow-hidden rounded-xl bg-neutral-900 shadow-2xl shadow-black/70 ring-1 ring-white/10"
      style={{ left, top, width: WIDTH }}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
      role="dialog"
      aria-label={title}
    >
      <button type="button" onClick={onOpen} tabIndex={-1} className="relative block aspect-video w-full overflow-hidden bg-neutral-800 outline-none">
        {image && <img src={image} alt="" className={`h-full w-full object-cover ${info.backdropPath ? "" : "object-top"}`} />}
        <span className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/90 to-transparent px-3 pb-2 pt-8 text-left text-base font-semibold text-white">{title}</span>
      </button>
      <div className="flex flex-col gap-2 px-3 pb-3 pt-2.5">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={info.onPlay ?? onOpen}
            className={`${round} bg-white text-black hover:bg-neutral-200`}
            aria-label={info.onPlay ? "Reproducir" : "Ver"}
            title={info.onPlay ? "Reproducir" : "Ver"}
          >
            <PlayIcon className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={() => toggleFavorite(info.kind, info.id, !inList)}
            aria-pressed={inList}
            className={`${round} border border-white/40 text-white hover:border-white`}
            aria-label={inList ? "Quitar de Mi lista" : "Añadir a Mi lista"}
            title={inList ? "Quitar de Mi lista" : "Añadir a Mi lista"}
          >
            <HeartIcon filled={inList} className={`h-4 w-4 ${inList ? "text-red-500" : ""}`} />
          </button>
          <button
            type="button"
            onClick={onOpen}
            className={`${round} ml-auto border border-white/40 text-white hover:border-white`}
            aria-label="Más información"
            title="Más información"
          >
            <ChevronIcon className="h-4 w-4 rotate-90" />
          </button>
        </div>
        <p className="text-xs text-neutral-300">{meta.join(" · ")}</p>
        {info.genres && info.genres.length > 0 && <p className="truncate text-xs text-neutral-400">{info.genres.join(" · ")}</p>}
      </div>
    </div>,
    document.body,
  );
}
