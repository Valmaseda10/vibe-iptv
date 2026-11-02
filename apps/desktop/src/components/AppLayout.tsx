import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { ProfileSwitcher } from "../features/profiles/ProfileSwitcher";
import { SyncStatusBanner } from "./SyncStatusBanner";
import { ReminderNotifier } from "./ReminderNotifier";
import { MobileTabBar } from "./MobileTabBar";
import { useSurpriseMe } from "../lib/surprise";
import { NAV_ITEMS, isNavActive } from "../lib/navigation";
import { SettingsIcon, ChevronIcon, DiceIcon } from "./icons";
import { NotificationScheduler } from "./NotificationScheduler";
import { WhatsNewSheet } from "./WhatsNewSheet";
import { useWhatsNewOnLaunch } from "../lib/releaseNotes";
import { BrandLockup, BrandMark } from "./BrandMark";


/** Cascarón de F5: barra lateral colapsable + contenido (en pantallas estrechas, una
 * barra de pestañas abajo: `MobileTabBar`, F15). Tab funciona como siempre;
 * arriba/abajo/derecha además mueven el foco a mano (ver `onNavKeyDown`, pensado para
 * mando a distancia). El foco siempre es visible (ver globals.css). */
export function AppLayout({ children }: { children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // Directo, Guía y «Catálogo completo» desplazan su propia lista y necesitan la altura justa
  // de la pantalla; el resto crece con su contenido, y así el hueco de la barra de pestañas
  // del iPhone queda al final de lo último (antes se quedaba tapado debajo).
  const fixedHeight = useRouterState({
    select: (s) => s.location.pathname === "/live" || s.location.pathname === "/guide" || (s.location.search as { view?: string }).view === "all",
  });
  const { surpriseMe, surprising } = useSurpriseMe();
  const [whatsNew, dismissWhatsNew] = useWhatsNewOnLaunch();
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

  // El reproductor va a pantalla completa, sin barra lateral ni cromo alrededor: es la
  // única pantalla de toda la app que no vive dentro de este layout de navegación.
  if (pathname.startsWith("/player/")) {
    return <div className="h-full bg-black">{children}</div>;
  }

  return (
    <div className="flex h-full flex-col bg-neutral-950 text-neutral-100 desk:flex-row ios:bg-black">
      <nav
        ref={navRef}
        onKeyDown={onNavKeyDown}
        className={`hidden shrink-0 flex-col border-r desk:flex border-neutral-900 bg-neutral-950 py-4 transition-[width] duration-150 ios:glass ios:my-3 ios:ml-3 ios:rounded-[22px] ${collapsed ? "w-16" : "w-56"}`}
        aria-label="Navegación principal"
      >
        <div className={`mb-3 flex items-center gap-2 px-4 ${collapsed ? "justify-center px-0" : ""}`}>
          {collapsed ? <BrandMark className="h-6 w-6" /> : <BrandLockup />}
        </div>

        <div className="mb-4">
          <ProfileSwitcher collapsed={collapsed} />
        </div>

        <ul className="flex flex-1 flex-col gap-1 px-2">
          {NAV_ITEMS.map(({ to, label, icon: ItemIcon, also }) => {
            const active = isNavActive(pathname, to, also);
            return (
              <li key={to}>
                <Link
                  to={to}
                  className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm outline-none transition-colors ${
                    active ? "bg-neutral-800 text-white ios:rounded-xl ios:bg-white/15" : "text-neutral-400 hover:bg-neutral-900 hover:text-neutral-100 ios:text-white/70"
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

        <Link
          to="/settings"
          className={`mx-2 mb-1 flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm outline-none transition-colors ${
            pathname.startsWith("/settings") ? "bg-neutral-800 text-white ios:rounded-xl ios:bg-white/15" : "text-neutral-400 hover:bg-neutral-900 hover:text-neutral-100 ios:text-white/70"
          } ${collapsed ? "justify-center px-0" : ""}`}
          aria-current={pathname.startsWith("/settings") ? "page" : undefined}
          title={collapsed ? "Ajustes" : undefined}
        >
          <SettingsIcon className="h-5 w-5 shrink-0" />
          {!collapsed && <span>Ajustes</span>}
        </Link>

        <button
          type="button"
          onClick={() => void surpriseMe()}
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

      <main ref={mainRef} data-shell-main className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div key={pathname} className={`page-enter ${fixedHeight ? "h-full" : "min-h-full"}`}>
          {children}
        </div>
      </main>
      <MobileTabBar />
      <SyncStatusBanner />
      <ReminderNotifier />
      <NotificationScheduler />
      {whatsNew && <WhatsNewSheet onClose={dismissWhatsNew} />}
    </div>
  );
}
