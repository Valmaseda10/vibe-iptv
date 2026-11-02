import { HeartIcon, HomeIcon, LiveIcon, SearchIcon, TrophyIcon } from "../components/icons";

/** Menú compacto (2026-11-02): Películas y Series viven dentro de Inicio (filtro
 * `Todo · Películas · Series`) y la Guía dentro de Directo (`Canales · Guía`). `also` son
 * las rutas que encienden la misma entrada. Mismo orden que las pestañas del iPhone. */
export const NAV_ITEMS = [
  { to: "/", label: "Inicio", icon: HomeIcon, also: ["/movies", "/series"] },
  { to: "/live", label: "Directo", icon: LiveIcon, also: ["/guide"] },
  { to: "/sports", label: "Deportes", icon: TrophyIcon, also: [] },
  { to: "/search", label: "Buscar", icon: SearchIcon, also: ["/collections", "/people"] },
  { to: "/favorites", label: "Mi lista", icon: HeartIcon, also: [] },
] as const;

/** ¿La entrada del menú `to` (con sus rutas `also`) es la pantalla actual? */
export function isNavActive(pathname: string, to: string, also: readonly string[]): boolean {
  const under = (base: string) => pathname === base || pathname.startsWith(`${base}/`);
  return (to === "/" ? pathname === "/" : under(to)) || also.some(under);
}
