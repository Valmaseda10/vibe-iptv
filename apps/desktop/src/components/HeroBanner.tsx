import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { imagePath, useApiBase } from "../lib/api-client";
import type { BrowseFeatured } from "../lib/api-types";
import { useFavorites } from "../lib/favorites";
import { IS_IOS } from "../lib/platform";
import { usePlayerSession } from "../lib/playerSession";
import { BackdropTrailer } from "./BackdropTrailer";
import { HeartIcon } from "./icons";

/** Portada grande de Inicio, Películas y Series, con el fondo del título destacado y por
 * qué está ahí («N.º 2 en España hoy», «Recomendada para ti»...). Se funde con el fondo de
 * la app para que las filas de debajo no choquen. */
export function HeroBanner({ featured }: { featured: BrowseFeatured }) {
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  const { isFavorite, toggleFavorite } = useFavorites();
  const backdrop = imagePath(base, featured.backdrop_path, "w1280");
  const isSeries = featured.kind === "series";
  const inList = isFavorite(featured.kind, featured.id);
  const isTop = featured.reason.startsWith("N.º");
  const heroRef = useRef<HTMLDivElement>(null);
  const trailerActive = useTrailerActive(heroRef);

  const openDetail = () =>
    isSeries
      ? navigate({ to: "/series/$seriesId", params: { seriesId: String(featured.id) } })
      : navigate({ to: "/movies/$movieId", params: { movieId: String(featured.id) } });

  const primary = IS_IOS
    ? "flex items-center gap-2 rounded-full bg-white px-6 py-3 text-[17px] font-semibold text-black outline-none active:bg-neutral-300"
    : "flex items-center gap-2 rounded-lg bg-white px-6 py-2.5 font-semibold text-black outline-none transition hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-red-600";
  const secondary = IS_IOS
    ? "glass-clear glass-pressable flex items-center gap-2 rounded-full px-5 py-3 text-[17px] font-semibold text-white outline-none"
    : "flex items-center gap-2 rounded-lg bg-neutral-700/80 px-5 py-2.5 font-semibold text-white outline-none transition hover:bg-neutral-600/80 focus-visible:ring-2 focus-visible:ring-red-600";

  return (
    <div ref={heroRef} className="relative h-[56vh] min-h-[340px] w-full overflow-hidden sm:h-[62vh] sm:min-h-[380px]" style={{ ["--trailer-h" as string]: "62vh" }}>
      {backdrop && <img key={backdrop} src={backdrop} alt="" className="hero-fade absolute inset-0 h-full w-full object-cover" />}
      {backdrop && featured.trailer_key && (
        <BackdropTrailer key={featured.trailer_key} youtubeKey={featured.trailer_key} desktop active={trailerActive} delayMs={2500} soundToggle />
      )}
      <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/40 to-transparent ios:from-black ios:via-black/40" />
      <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/85 via-neutral-950/20 to-transparent ios:from-black/60" />

      <div className="absolute bottom-0 left-0 flex max-w-xl flex-col gap-3 px-4 pb-8 sm:gap-4 sm:px-10 sm:pb-10">
        <p className="flex items-center gap-2 text-sm font-semibold text-neutral-100 drop-shadow">
          {isTop && <span className="rounded bg-red-600 px-1.5 py-0.5 text-[11px] font-bold leading-none tracking-wide text-white">TOP 10</span>}
          {featured.reason}
        </p>
        <h2 className="text-3xl font-bold drop-shadow-lg sm:text-5xl">{featured.clean_title}</h2>
        <p className="flex items-center gap-3 text-sm text-neutral-300">
          {featured.year && <span>{featured.year}</span>}
          {featured.rating != null && featured.rating > 0 && <span>★ {featured.rating.toFixed(1).replace(".", ",")}</span>}
          <span>{isSeries ? "Serie" : "Película"}</span>
        </p>
        {featured.overview && <p className="line-clamp-3 text-sm text-neutral-200 drop-shadow sm:text-base">{featured.overview}</p>}
        <div className="flex flex-wrap items-center gap-3">
          {isSeries ? (
            <button type="button" onClick={openDetail} className={primary}>
              ▶ Ver serie
            </button>
          ) : (
            <button type="button" onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(featured.id) } })} className={primary}>
              ▶ Reproducir
            </button>
          )}
          <button type="button" onClick={() => toggleFavorite(featured.kind, featured.id, !inList)} aria-pressed={inList} className={secondary}>
            <HeartIcon filled={inList} className={`h-5 w-5 ${inList ? "text-red-500" : ""}`} />
            Mi lista
          </button>
          {!isSeries && (
            <button type="button" onClick={openDetail} className={secondary}>
              Más información
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/** El tráiler de la portada solo corre con la portada a la vista, la ventana delante y
 * nada sonando en el reproductor (ni en pequeño). */
function useTrailerActive(ref: React.RefObject<HTMLDivElement | null>): boolean {
  const { session } = usePlayerSession();
  const [inView, setInView] = useState(true);
  const [pageVisible, setPageVisible] = useState(() => !document.hidden);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new IntersectionObserver(([entry]) => setInView((entry?.intersectionRatio ?? 0) >= 0.4), { threshold: [0, 0.4, 1] });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  useEffect(() => {
    const onChange = () => setPageVisible(!document.hidden);
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  return inView && pageVisible && session === null;
}

export function HeroBannerSkeleton() {
  return (
    <div className="relative flex h-[56vh] min-h-[340px] w-full animate-pulse items-end overflow-hidden bg-neutral-900 sm:h-[62vh] sm:min-h-[380px]">
      <div className="flex w-full max-w-xl flex-col gap-4 px-4 pb-8 sm:px-10 sm:pb-10">
        <div className="h-4 w-40 rounded bg-neutral-800" />
        <div className="h-10 w-2/3 rounded bg-neutral-800" />
        <div className="h-4 w-full rounded bg-neutral-800" />
        <div className="h-4 w-5/6 rounded bg-neutral-800" />
        <div className="h-10 w-48 rounded bg-neutral-800" />
      </div>
    </div>
  );
}
