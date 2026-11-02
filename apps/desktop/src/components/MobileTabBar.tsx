import { useEffect, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { IS_IOS } from "../lib/platform";
import { NAV_ITEMS, isNavActive } from "../lib/navigation";
import { ProfileAvatar } from "./ProfileAvatar";

/** Barra de pestañas inferior para pantallas estrechas (iPhone, F15): sustituye a la barra
 * lateral, que en vertical se comería media pantalla. Desde 2026-11-02 son las mismas cinco
 * entradas que la barra lateral y ya no hay «Más»: Ajustes, «Tu año», «Sorpréndeme» y
 * «Cambiar de perfil» están arriba de «Mi lista», cuya pestaña lleva el avatar del perfil.
 *
 * En el PC estrecho va en el flujo de la página (último hijo de la columna de `AppLayout`).
 * En iOS es una cápsula de cristal que flota sobre el contenido, como en iOS 26, con una
 * "gota" que se desliza a la pestaña activa; mientras se escribe se esconde, para que el
 * teclado no la arrastre (lo fijo se quedaba desplazado al cerrarlo). */
function isTextField(target: EventTarget | null): boolean {
  if (target instanceof HTMLTextAreaElement) return true;
  // Solo los campos que sacan el teclado (no casillas, botones ni deslizadores).
  return target instanceof HTMLInputElement && ["text", "search", "email", "password", "url", "tel", "number"].includes(target.type);
}

export function MobileTabBar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const [typing, setTyping] = useState(false);
  // Un campo que desaparece con el teclado abierto (cambio de pantalla) no siempre avisa
  // con `focusout`: al cambiar de pantalla se vuelve a mirar quién tiene el foco.
  useEffect(() => setTyping(IS_IOS && isTextField(document.activeElement)), [pathname]);
  useEffect(() => {
    if (!IS_IOS) return;
    const isField = isTextField;
    const onIn = (e: FocusEvent) => isField(e.target) && setTyping(true);
    const onOut = (e: FocusEvent) => isField(e.target) && setTyping(false);
    document.addEventListener("focusin", onIn);
    document.addEventListener("focusout", onOut);
    return () => {
      document.removeEventListener("focusin", onIn);
      document.removeEventListener("focusout", onOut);
    };
  }, []);

  const tabClass = (active: boolean) =>
    IS_IOS
      ? `relative z-10 flex flex-1 flex-col items-center justify-center gap-0.5 text-[10px] font-medium outline-none ${active ? "text-white" : "text-white/60"}`
      : `flex flex-1 flex-col items-center gap-0.5 py-1.5 text-[10px] outline-none ${active ? "text-white" : "text-neutral-500"}`;
  // Ajustes y «Tu año» se abren desde «Mi lista»: esa pestaña sigue encendida.
  const inMyList = pathname.startsWith("/settings") || pathname.startsWith("/year");
  const activeIndex = NAV_ITEMS.findIndex(({ to, also }) => (to === "/favorites" && inMyList) || isNavActive(pathname, to, also));

  return (
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
          style={{ width: `calc((100% - 0.5rem) / ${NAV_ITEMS.length})`, transform: `translateX(${activeIndex * 100}%)` }}
        />
      )}
      {NAV_ITEMS.map(({ to, label, icon: ItemIcon }, index) => {
        const active = index === activeIndex;
        return (
          <Link key={to} to={to} aria-label={label} className={tabClass(active)} aria-current={active ? "page" : undefined}>
            {to === "/favorites" ? (
              <ProfileAvatar className={`h-6 w-6 text-[13px] ${active ? "ring-2 ring-white" : "opacity-80"}`} />
            ) : (
              <ItemIcon className="h-6 w-6" />
            )}
            {label}
          </Link>
        );
      })}
    </nav>
  );
}
