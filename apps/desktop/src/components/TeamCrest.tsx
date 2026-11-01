import { crestColor, crestInitials } from "../lib/sports";

/** Escudo dibujado de un equipo: sus iniciales sobre su color (la guía no trae escudos). */
export function TeamCrest({ teamKey, name, size = "md" }: { teamKey: string; name: string; size?: "sm" | "md" | "lg" }) {
  const color = crestColor(teamKey);
  const light = /^#(e|f)/i.test(color);
  const sizes = { sm: "h-8 w-8 text-[11px]", md: "h-11 w-11 text-sm", lg: "h-16 w-16 text-lg sm:h-20 sm:w-20 sm:text-xl" };
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
