import { useEffect, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useActiveProfile } from "../lib/activeProfile";
import { IS_IOS } from "../lib/platform";
import { GlassSheet, SHEET_ROW } from "./GlassSheet";
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
 * En el PC estrecho va en el flujo de la página (último hijo de la columna de `AppLayout`).
 * En iOS es una cápsula de cristal que flota sobre el contenido, como en iOS 26, con una
 * "gota" que se desliza a la pestaña activa; mientras se escribe se esconde, para que el
 * teclado no la arrastre (lo fijo se quedaba desplazado al cerrarlo). */
export function MobileTabBar({ onSurprise, surprising }: { onSurprise: () => void; surprising: boolean }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { setProfileId } = useActiveProfile();
  const [moreOpen, setMoreOpen] = useState(false);
  const moreActive = MORE.some(({ to }) => pathname.startsWith(to));

  useEffect(() => setMoreOpen(false), [pathname]);

  const [typing, setTyping] = useState(false);
  useEffect(() => {
    if (!IS_IOS) return;
    const isField = (t: EventTarget | null) => t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement;
    const onIn = (e: FocusEvent) => isField(e.target) && setTyping(true);
    const onOut = (e: FocusEvent) => isField(e.target) && setTyping(false);
    document.addEventListener("focusin", onIn);
    document.addEventListener("focusout", onOut);
    return () => {
      document.removeEventListener("focusin", onIn);
      document.removeEventListener("focusout", onOut);
    };
  }, []);

  useEffect(() => {
    if (!moreOpen) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") setMoreOpen(false);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [moreOpen]);

  const tabClass = (active: boolean) =>
    IS_IOS
      ? `relative z-10 flex flex-1 flex-col items-center justify-center gap-0.5 text-[10px] font-medium outline-none ${active ? "text-white" : "text-white/60"}`
      : `flex flex-1 flex-col items-center gap-0.5 py-1.5 text-[10px] outline-none ${active ? "text-white" : "text-neutral-500"}`;
  const activeIndex = moreActive || moreOpen ? TABS.length : TABS.findIndex(({ to }) => (to === "/" ? pathname === "/" : pathname.startsWith(to)));

  const moreItems = (rowClass: string, iconClass: string) => (
    <>
      {MORE.map(({ to, label, icon: ItemIcon }) => (
        <Link key={to} to={to} role="menuitem" className={rowClass}>
          <ItemIcon className={iconClass} />
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
        className={rowClass}
      >
        <DiceIcon className={iconClass} />
        Sorpréndeme
      </button>
      <button
        type="button"
        role="menuitem"
        onClick={() => {
          setMoreOpen(false);
          setProfileId(null);
        }}
        className={rowClass}
      >
        <span className={`flex items-center justify-center rounded-full border border-current text-[10px] ${iconClass}`}>↺</span>
        Cambiar de perfil
      </button>
    </>
  );

  return (
    <>
      {moreOpen && IS_IOS && (
        <GlassSheet title="Más" onClose={() => setMoreOpen(false)}>
          <div role="menu" aria-label="Más opciones">
            {moreItems(SHEET_ROW, "h-6 w-6")}
          </div>
        </GlassSheet>
      )}
      {moreOpen && !IS_IOS && (
        <div className="fixed inset-0 z-40 bg-black/60" onClick={() => setMoreOpen(false)} aria-hidden="true" />
      )}
      {moreOpen && !IS_IOS && (
        <div
          role="menu"
          aria-label="Más opciones"
          className="fixed inset-x-0 bottom-[calc(3.5rem+env(safe-area-inset-bottom))] z-50 mx-3 mb-2 rounded-xl border border-neutral-800 bg-neutral-900 p-2 shadow-2xl"
        >
          {moreItems("flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left text-sm text-neutral-200 outline-none active:bg-neutral-800 disabled:opacity-50", "h-5 w-5")}
        </div>
      )}

      <nav
        aria-label="Navegación principal"
        className={
          IS_IOS
            ? `glass fixed inset-x-4 bottom-[max(10px,calc(env(safe-area-inset-bottom)-12px))] z-50 mx-auto flex h-[62px] max-w-md rounded-full p-1 transition-[transform,opacity] duration-300 ease-[var(--ease-smooth)] desk:hidden ${typing ? "pointer-events-none translate-y-[150%] opacity-0" : ""}`
            : "relative z-50 flex shrink-0 border-t border-neutral-900 bg-neutral-950 pb-[env(safe-area-inset-bottom)] desk:hidden"
        }
      >
        {IS_IOS && activeIndex >= 0 && (
          // La "gota": se desliza bajo la pestaña activa (solo `transform`, barato).
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-y-1 left-1 rounded-full bg-white/15 transition-transform duration-[350ms] ease-[var(--ease-spring)]"
            style={{ width: `calc((100% - 0.5rem) / ${TABS.length + 1})`, transform: `translateX(${activeIndex * 100}%)` }}
          />
        )}
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
