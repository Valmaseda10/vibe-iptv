import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { ProfileSwitcher } from "../features/profiles/ProfileSwitcher";
import { SyncStatusBanner } from "./SyncStatusBanner";
import { useActiveProfile } from "../lib/activeProfile";
import { ReminderNotifier } from "./ReminderNotifier";
import { MobileTabBar } from "./MobileTabBar";
import { apiGetOrNull } from "../lib/api-client";
import type { RandomTitleResponse } from "../lib/api-types";
import { HomeIcon, LiveIcon, GuideIcon, MovieIcon, SeriesIcon, SearchIcon, HeartIcon, SettingsIcon, ChevronIcon, DiceIcon } from "./icons";

const NAV_ITEMS = [
  { to: "/", label: "Inicio", icon: HomeIcon },
  { to: "/live", label: "Directo", icon: LiveIcon },
  { to: "/guide", label: "Guía", icon: GuideIcon },
  { to: "/movies", label: "Películas", icon: MovieIcon },
  { to: "/series", label: "Series", icon: SeriesIcon },
  { to: "/search", label: "Buscar", icon: SearchIcon },
  { to: "/favorites", label: "Favoritos", icon: HeartIcon },
  { to: "/settings", label: "Ajustes", icon: SettingsIcon },
] as const;

/** Cascarón de F5: barra lateral colapsable + contenido (en pantallas estrechas, una
 * barra de pestañas abajo: `MobileTabBar`, F15). Tab funciona como siempre;
 * arriba/abajo/derecha además mueven el foco a mano (ver `onNavKeyDown`, pensado para
 * mando a distancia). El foco siempre es visible (ver globals.css). */
export function AppLayout({ children }: { children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const [surprising, setSurprising] = useState(false);
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const navRef = useRef<HTMLElement>(null);
  const mainRef = useRef<HTMLElement>(null);

  // Arriba/abajo mueve el foco entre los enlaces y botones de la barra lateral, y
  // derecha entra al contenido (F "premium": pensado para mando a distancia, que no
  // tiene Tab — ver también Row.tsx para el resto de la navegación por flechas).
  function onNavKeyDown(e: React.KeyboardEvent<HTMLElement>) {
    const nav = navRef.current;
    if (!nav) return;
    const focusable = Array.from(nav.querySelectorAll<HTMLElement>("a, button"));
    const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
    if (currentIndex === -1) return;

    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const nextIndex = e.key === "ArrowDown" ? Math.min(currentIndex + 1, focusable.length - 1) : Math.max(currentIndex - 1, 0);
      focusable[nextIndex]?.focus();
    } else if (e.key === "ArrowRight") {
      const target = mainRef.current?.querySelector<HTMLElement>("a, button, input, select, textarea, [tabindex]");
      if (!target) return;
      e.preventDefault();
      target.focus();
    }
  }

  // Cada pantalla empieza arriba: antes se heredaba el desplazamiento de la anterior.
  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0 });
  }, [pathname]);

  // Al abrir la app, o tras una navegación que desmonta el elemento enfocado, el foco
  // queda en <body> y las flechas no hacían nada: con un mando no hay Tab para empezar.
  // La primera flecha enfoca el primer elemento del contenido.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (!e.key.startsWith("Arrow")) return;
      if (document.activeElement && document.activeElement !== document.body) return;
      const target = mainRef.current?.querySelector<HTMLElement>("[data-card], a, button, input, select, textarea, [tabindex]");
      if (!target) return;
      e.preventDefault();
      target.focus();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // "Sorpréndeme" (F "premium"): lleva directo a la ficha de un título al azar, nunca
  // abre el reproductor solo — sigue siendo el usuario quien decide reproducir (R1).
  async function surpriseMe() {
    if (surprising) return;
    setSurprising(true);
    try {
      if (profileId === null) return;
      const pick = await apiGetOrNull<RandomTitleResponse>(`/api/random?profile_id=${profileId}`);
      if (!pick) return;
      if (pick.kind === "series") navigate({ to: "/series/$seriesId", params: { seriesId: String(pick.id) } });
      else navigate({ to: "/movies/$movieId", params: { movieId: String(pick.id) } });
    } finally {
      setSurprising(false);
    }
  }

  // El reproductor va a pantalla completa, sin barra lateral ni cromo alrededor: es la
  // única pantalla de toda la app que no vive dentro de este layout de navegación.
  if (pathname.startsWith("/player/")) {
    return <div className="h-full bg-black">{children}</div>;
  }

  return (
    <div className="flex h-full flex-col bg-neutral-950 text-neutral-100 desk:flex-row">
      <nav
        ref={navRef}
        onKeyDown={onNavKeyDown}
        className={`hidden shrink-0 flex-col border-r desk:flex border-neutral-900 bg-neutral-950 py-4 transition-[width] duration-150 ${collapsed ? "w-16" : "w-56"}`}
        aria-label="Navegación principal"
      >
        <div className={`mb-3 flex items-center gap-2 px-4 ${collapsed ? "justify-center px-0" : ""}`}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full bg-red-600" />
          {!collapsed && <span className="text-sm font-semibold tracking-wide">VIBE IPTV</span>}
        </div>

        <div className="mb-4">
          <ProfileSwitcher collapsed={collapsed} />
        </div>

        <ul className="flex flex-1 flex-col gap-1 px-2">
          {NAV_ITEMS.map(({ to, label, icon: ItemIcon }) => {
            const active = to === "/" ? pathname === "/" : pathname.startsWith(to);
            return (
              <li key={to}>
                <Link
                  to={to}
                  className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm outline-none transition-colors ${
                    active ? "bg-neutral-800 text-white" : "text-neutral-400 hover:bg-neutral-900 hover:text-neutral-100"
                  } ${collapsed ? "justify-center px-0" : ""}`}
                  aria-current={active ? "page" : undefined}
                  title={collapsed ? label : undefined}
                >
                  <ItemIcon className="h-5 w-5 shrink-0" />
                  {!collapsed && <span>{label}</span>}
                </Link>
              </li>
            );
          })}
        </ul>

        <button
          type="button"
          onClick={surpriseMe}
          disabled={surprising}
          className={`mx-2 mb-2 flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm text-neutral-400 outline-none transition-colors hover:bg-neutral-900 hover:text-neutral-100 disabled:opacity-50 ${
            collapsed ? "justify-center px-0" : ""
          }`}
          title={collapsed ? "Sorpréndeme" : undefined}
        >
          <DiceIcon className={`h-5 w-5 shrink-0 ${surprising ? "animate-spin" : ""}`} />
          {!collapsed && <span>Sorpréndeme</span>}
        </button>

        <button
          type="button"
          onClick={() => setCollapsed((c) => !c)}
          className={`mx-2 flex items-center justify-center gap-2 rounded-lg py-2 text-neutral-500 outline-none hover:bg-neutral-900 hover:text-neutral-200`}
          aria-label={collapsed ? "Expandir menú" : "Colapsar menú"}
        >
          <ChevronIcon className={`h-4 w-4 transition-transform ${collapsed ? "" : "rotate-180"}`} />
        </button>
      </nav>

      <main ref={mainRef} className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div key={pathname} className="page-enter h-full">
          {children}
        </div>
      </main>
      <MobileTabBar onSurprise={surpriseMe} surprising={surprising} />
      <SyncStatusBanner />
      <ReminderNotifier />
    </div>
  );
}
