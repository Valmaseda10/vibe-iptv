import { useState } from "react";
import { crestColor, crestInitials } from "../lib/sports";

/** Escudo de un equipo: el de verdad (ESPN) si se conoce y carga; si no, sus iniciales
 * sobre su color (la guía no trae escudos). */
export function TeamCrest({ teamKey, name, logo, size = "md" }: { teamKey: string; name: string; logo?: string | null; size?: "sm" | "md" | "lg" | "xl" }) {
  const [failed, setFailed] = useState(false);
  const sizes = {
    sm: "h-8 w-8 text-[11px]",
    md: "h-11 w-11 text-sm",
    lg: "h-16 w-16 text-lg sm:h-20 sm:w-20 sm:text-xl",
    xl: "h-20 w-20 text-xl sm:h-28 sm:w-28 sm:text-2xl",
  };
  if (logo && !failed) {
    return (
      <span aria-hidden className={`flex shrink-0 items-center justify-center ${sizes[size]}`}>
        <img src={logo} alt="" onError={() => setFailed(true)} className="h-full w-full object-contain drop-shadow-[0_6px_14px_rgba(0,0,0,0.55)]" />
      </span>
    );
  }
  const color = crestColor(teamKey);
  const light = /^#(e|f)/i.test(color);
  return (
    <span
      aria-hidden
      className={`flex shrink-0 items-center justify-center rounded-full font-bold tracking-tight shadow-lg ring-2 ring-white/20 ${sizes[size]} ${light ? "text-neutral-900" : "text-white"}`}
      style={{ background: `radial-gradient(circle at 30% 25%, rgba(255,255,255,0.35), transparent 55%), ${color}` }}
    >
      {crestInitials(name)}
    </span>
  );
}
