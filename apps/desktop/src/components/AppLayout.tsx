import { useState, type ReactNode } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { HomeIcon, LiveIcon, GuideIcon, MovieIcon, SeriesIcon, SearchIcon, SettingsIcon, ChevronIcon } from "./icons";

const NAV_ITEMS = [
  { to: "/", label: "Inicio", icon: HomeIcon },
  { to: "/live", label: "Directo", icon: LiveIcon },
  { to: "/guide", label: "Guía", icon: GuideIcon },
  { to: "/movies", label: "Películas", icon: MovieIcon },
  { to: "/series", label: "Series", icon: SeriesIcon },
  { to: "/search", label: "Buscar", icon: SearchIcon },
  { to: "/settings", label: "Ajustes", icon: SettingsIcon },
] as const;

/** Cascarón de F5: barra lateral colapsable + contenido. Navegación con Tab/flechas
 * nativa de los enlaces; el foco siempre es visible (ver globals.css). */
export function AppLayout({ children }: { children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  // El reproductor va a pantalla completa, sin barra lateral ni cromo alrededor: es la
  // única pantalla de toda la app que no vive dentro de este layout de navegación.
  if (pathname.startsWith("/player/")) {
    return <div className="h-full bg-black">{children}</div>;
  }

  return (
    <div className="flex h-full bg-neutral-950 text-neutral-100">
      <nav
        className={`flex shrink-0 flex-col border-r border-neutral-900 bg-neutral-950 py-4 transition-[width] duration-150 ${collapsed ? "w-16" : "w-56"}`}
        aria-label="Navegación principal"
      >
        <div className={`mb-6 flex items-center gap-2 px-4 ${collapsed ? "justify-center px-0" : ""}`}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full bg-red-600" />
          {!collapsed && <span className="text-sm font-semibold tracking-wide">VIBE IPTV</span>}
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
          onClick={() => setCollapsed((c) => !c)}
          className={`mx-2 flex items-center justify-center gap-2 rounded-lg py-2 text-neutral-500 outline-none hover:bg-neutral-900 hover:text-neutral-200`}
          aria-label={collapsed ? "Expandir menú" : "Colapsar menú"}
        >
          <ChevronIcon className={`h-4 w-4 transition-transform ${collapsed ? "" : "rotate-180"}`} />
        </button>
      </nav>

      <main className="min-w-0 flex-1 overflow-y-auto">{children}</main>
    </div>
  );
}
