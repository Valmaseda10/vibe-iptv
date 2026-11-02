import { useEffect, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { BrowseResponse } from "../../lib/api-types";
import { feedVisit, rememberFeatured, reshuffleVisit } from "../../lib/feedSeed";
import { HeroBanner, HeroBannerSkeleton } from "../../components/HeroBanner";
import { PosterCardSkeleton } from "../../components/PosterCard";
import { BrowseRows } from "./BrowseRows";
import { MatchHero } from "../sports/MatchHero";
import { useFeaturedMatch } from "../../lib/sports";
import { HOME_SECTIONS, SectionSwitch } from "../../components/SectionSwitch";

type BrowseScreenProps = {
  page: "movies" | "series" | "home";
  /** Nombre de la pantalla, arriba a la izquierda sobre la portada. */
  title: string;
  /** Botones arriba, junto al nombre (p. ej. «Catálogo completo»). */
  actions?: ReactNode;
  /** Filas propias que van justo debajo de la portada (Inicio: seguir viendo, canales...). */
  before?: ReactNode;
};

/** Inicio, Películas y Series como en Netflix (con el filtro `Todo · Películas · Series`
 * arriba): portada y filas que cambian en cada visita
 * (ver `catalog::browse` en el núcleo y `lib/feedSeed`). Igual en el PC y en iPhone/iPad. */
export function BrowseScreen({ page, title, actions, before }: BrowseScreenProps) {
  const { profileId } = useActiveProfile();
  // Una semilla y unas portadas a evitar por visita (ver `lib/feedSeed`).
  const [{ seed, avoid }, setVisit] = useState(feedVisit);
  const query = useQuery({
    queryKey: ["browse", page, profileId, seed],
    queryFn: () => apiGet<BrowseResponse>(`/api/browse/${page}?profile_id=${profileId}&seed=${seed}${avoid ? `&avoid=${encodeURIComponent(avoid)}` : ""}`),
    enabled: profileId !== null,
    // Al volver a la pantalla se vuelve a pedir con la misma semilla: mismas filas, pero al
    // día (lo que se acaba de ver o añadir a la lista).
    // La primera vez el núcleo aún está calculando las recomendaciones del perfil.
    refetchInterval: (q) => (q.state.data?.pending && q.state.dataUpdateCount < 6 ? 3_000 : false),
  });
  const featured = query.data?.featured;
  // Inicio: un partido importante (de sus equipos, o un Madrid, un Barça, la Champions...)
  // ocupa la portada mientras está en juego o a punto (F17), como en Movistar Plus.
  const match = useFeaturedMatch();
  const featuredMatch = page === "home" ? match.data : null;
  useEffect(() => {
    if (featured) rememberFeatured(featured.kind, featured.id);
  }, [featured]);

  return (
    <div className="relative flex flex-col gap-8 pb-10 sm:gap-10">
      <div className="absolute inset-x-0 top-0 z-20 flex flex-wrap items-center gap-3 px-4 pt-4 sm:px-10 sm:pt-6">
        <h1 className="text-2xl font-bold text-white drop-shadow-lg sm:text-3xl ios:text-[34px] ios:tracking-tight">{title}</h1>
        {/* Películas y Series viven dentro de Inicio (2026-11-02), como el filtro de Netflix. */}
        <SectionSwitch items={HOME_SECTIONS} label="Qué ver" />
        {actions}
      </div>

      {featuredMatch ? (
        <MatchHero event={featuredMatch} />
      ) : query.isPending ? (
        <HeroBannerSkeleton />
      ) : featured ? (
        <HeroBanner featured={featured} />
      ) : (
        <div className="h-16 sm:h-20" />
      )}

      {before}

      {query.isError && <p className="px-4 text-red-400 sm:px-10">No se pudo cargar esta pantalla. ¿Hay algún proveedor sincronizado?</p>}

      {query.isPending ? (
        Array.from({ length: 3 }).map((_, i) => (
          <section key={i} className="flex flex-col gap-3">
            <div className="mx-4 h-5 w-48 skeleton rounded sm:mx-10" />
            <div className="flex gap-4 overflow-hidden px-4 sm:px-10">
              {Array.from({ length: 7 }).map((_, j) => (
                <PosterCardSkeleton key={j} />
              ))}
            </div>
          </section>
        ))
      ) : (
        <BrowseRows rows={query.data?.rows ?? []} page={page} />
      )}

      {query.isSuccess && (query.data.rows.length > 0 || query.data.featured) && (
        <div className="flex flex-col items-center gap-3 px-4 pt-4 text-center sm:px-10">
          <p className="text-sm text-neutral-400">¿Nada te convence?</p>
          <button
            type="button"
            onClick={() => {
              setVisit(reshuffleVisit(shownTitles(query.data)));
              document.querySelector("[data-shell-main]")?.scrollTo({ top: 0, behavior: "smooth" });
            }}
            className="flex items-center gap-2 rounded-full bg-white/10 px-5 py-2.5 text-sm font-semibold text-white outline-none ring-1 ring-white/10 transition-colors hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white/80"
          >
            <span aria-hidden className="text-base leading-none">↻</span>
            Ver otras
          </button>
        </div>
      )}
    </div>
  );
}

/** Filas que no se barajan: el Top 10 es un ranking y Mi lista y lo empezado son del perfil. */
const FIXED_ROWS = new Set(["top", "my-list", "continue", "new-episodes"]);

/** Lo que hay ahora en pantalla (portada y filas que se barajan), para no repetirlo. */
function shownTitles(data: BrowseResponse): string[] {
  const keys = data.rows
    .filter((row) => !FIXED_ROWS.has(row.key))
    .flatMap((row) => row.items.filter((item) => item.kind === "movie" || item.kind === "series").map((item) => `${item.kind}:${item.id}`));
  if (data.featured) keys.push(`${data.featured.kind}:${data.featured.id}`);
  return keys;
}
