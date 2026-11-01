import { useNavigate } from "@tanstack/react-router";
import { imagePath, useApiBase } from "../lib/api-client";
import type { CreditSummary } from "../lib/api-types";
import { Row } from "./Row";

/** Iniciales para quien no tiene foto en TMDB. */
function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? "")
    .join("");
}

/** Reparto y dirección de una ficha (F17): caras redondas con nombre y personaje; abrir
 * una lleva a su ficha con lo suyo que hay en el catálogo. */
export function CastRow({ cast }: { cast: CreditSummary[] }) {
  const navigate = useNavigate();
  const { data: base } = useApiBase();
  if (cast.length === 0) return null;
  return (
    <Row title="Reparto">
      {cast.map((person) => {
        const photo = imagePath(base, person.profile_path, "w185");
        const role = person.role === "director" ? "Dirección" : person.role === "creator" ? "Creación" : person.character;
        return (
          <button
            key={`${person.person_id}-${person.role}`}
            type="button"
            data-card
            onClick={() => navigate({ to: "/people/$personId", params: { personId: String(person.person_id) } })}
            className="group flex w-24 shrink-0 flex-col items-center gap-2 rounded-xl p-1 text-center outline-none focus-visible:ring-2 focus-visible:ring-white/70 sm:w-28"
          >
            <span className="relative block h-20 w-20 overflow-hidden rounded-full bg-neutral-800 ring-1 ring-white/10 transition-transform duration-200 group-hover:scale-105 group-focus-visible:scale-105 sm:h-24 sm:w-24">
              {photo ? (
                <img src={photo} alt="" loading="lazy" className="h-full w-full object-cover" />
              ) : (
                <span className="flex h-full w-full items-center justify-center text-lg font-semibold text-neutral-400">{initials(person.name)}</span>
              )}
            </span>
            <span className="line-clamp-2 text-xs font-medium leading-tight text-neutral-100 sm:text-sm">{person.name}</span>
            {role && <span className="-mt-1 line-clamp-2 text-[11px] leading-tight text-neutral-500 sm:text-xs">{role}</span>}
          </button>
        );
      })}
    </Row>
  );
}
