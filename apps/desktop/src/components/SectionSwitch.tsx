import { Link, useRouterState } from "@tanstack/react-router";

type SectionLink = { to: "/" | "/movies" | "/series" | "/live" | "/guide"; label: string };

/** Filtro de secciones arriba de una pantalla (2026-11-02, pedido por el usuario): `Todo ·
 * Películas · Series` en Inicio, como Netflix, y `Canales · Guía` en Directo. Cada botón
 * es una pantalla; el activo, en blanco. Se maneja con teclado (Tab y Enter). */
export function SectionSwitch({ items, label }: { items: SectionLink[]; label: string }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  return (
    <nav aria-label={label} className="flex shrink-0 items-center gap-1.5 rounded-full bg-black/55 p-1 backdrop-blur-md ios:bg-black/45">
      {items.map(({ to, label: text }) => {
        const active = to === "/" ? pathname === "/" : pathname === to || pathname.startsWith(`${to}/`);
        return (
          <Link
            key={to}
            to={to}
            aria-current={active ? "page" : undefined}
            className={`rounded-full px-3.5 py-1.5 text-sm font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/80 ${
              active ? "bg-white text-black" : "text-white/85 hover:bg-white/15"
            }`}
          >
            {text}
          </Link>
        );
      })}
    </nav>
  );
}

export const HOME_SECTIONS: SectionLink[] = [
  { to: "/", label: "Todo" },
  { to: "/movies", label: "Películas" },
  { to: "/series", label: "Series" },
];

export const LIVE_SECTIONS: SectionLink[] = [
  { to: "/live", label: "Canales" },
  { to: "/guide", label: "Guía" },
];
