import { useEffect, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useActiveProfile } from "../lib/activeProfile";
import { DiceIcon, GuideIcon, HeartIcon, HomeIcon, LiveIcon, MoreIcon, MovieIcon, SearchIcon, SeriesIcon, SettingsIcon } from "./icons";

const TABS = [
  { to: "/", label: "Inicio", icon: HomeIcon },
  { to: "/live", label: "Directo", icon: LiveIcon },
  { to: "/movies", label: "Películas", icon: MovieIcon },
  { to: "/series", label: "Series", icon: SeriesIcon },
  { to: "/search", label: "Buscar", icon: SearchIcon },
] as const;

const MORE = [
  { to: "/guide", label: "Guía", icon: GuideIcon },
  { to: "/favorites", label: "Favoritos", icon: HeartIcon },
  { to: "/settings", label: "Ajustes", icon: SettingsIcon },
] as const;

/** Barra de pestañas inferior para pantallas estrechas (iPhone, F15): sustituye a la barra
 * lateral, que en vertical se comería media pantalla. Lo que no cabe va en "Más".
 *
 * Va en el flujo de la página (último hijo de la columna de `AppLayout`), no fija con
 * `position: fixed`: en el WebView de iOS, tras cerrar el teclado, lo fijo se quedaba
 * desplazado y dejaba un hueco debajo. */
export function MobileTabBar({ onSurprise, surprising }: { onSurprise: () => void; surprising: boolean }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { setProfileId } = useActiveProfile();
  const [moreOpen, setMoreOpen] = useState(false);
  const moreActive = MORE.some(({ to }) => pathname.startsWith(to));

  useEffect(() => setMoreOpen(false), [pathname]);

  useEffect(() => {
    if (!moreOpen) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") setMoreOpen(false);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [moreOpen]);

  const tabClass = (active: boolean) =>
    `flex flex-1 flex-col items-center gap-0.5 py-1.5 text-[10px] outline-none ${active ? "text-white" : "text-neutral-500"}`;

  return (
    <>
      {moreOpen && (
        <div className="fixed inset-0 z-40 bg-black/60" onClick={() => setMoreOpen(false)} aria-hidden="true" />
      )}
      {moreOpen && (
        <div
          role="menu"
          aria-label="Más opciones"
          className="fixed inset-x-0 bottom-[calc(3.5rem+env(safe-area-inset-bottom))] z-50 mx-3 mb-2 rounded-xl border border-neutral-800 bg-neutral-900 p-2 shadow-2xl"
        >
          {MORE.map(({ to, label, icon: ItemIcon }) => (
            <Link key={to} to={to} role="menuitem" className="flex items-center gap-3 rounded-lg px-3 py-3 text-sm text-neutral-200 outline-none active:bg-neutral-800">
              <ItemIcon className="h-5 w-5" />
              {label}
            </Link>
          ))}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMoreOpen(false);
              onSurprise();
            }}
            disabled={surprising}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left text-sm text-neutral-200 outline-none active:bg-neutral-800 disabled:opacity-50"
          >
            <DiceIcon className="h-5 w-5" />
            Sorpréndeme
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMoreOpen(false);
              setProfileId(null);
            }}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left text-sm text-neutral-200 outline-none active:bg-neutral-800"
          >
            <span className="flex h-5 w-5 items-center justify-center rounded-full border border-neutral-500 text-[10px]">↺</span>
            Cambiar de perfil
          </button>
        </div>
      )}

      <nav
        aria-label="Navegación principal"
        className="relative z-50 flex shrink-0 border-t border-neutral-900 bg-neutral-950 pb-[env(safe-area-inset-bottom)] desk:hidden"
      >
        {TABS.map(({ to, label, icon: ItemIcon }) => {
          const active = to === "/" ? pathname === "/" : pathname.startsWith(to);
          return (
            <Link key={to} to={to} className={tabClass(active)} aria-current={active ? "page" : undefined}>
              <ItemIcon className="h-6 w-6" />
              {label}
            </Link>
          );
        })}
        <button type="button" onClick={() => setMoreOpen((o) => !o)} className={tabClass(moreActive || moreOpen)} aria-expanded={moreOpen} aria-haspopup="menu">
          <MoreIcon className="h-6 w-6" />
          Más
        </button>
      </nav>
    </>
  );
}
