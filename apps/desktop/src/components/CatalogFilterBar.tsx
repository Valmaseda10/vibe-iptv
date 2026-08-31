import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../lib/api-client";
import type { GenreSummary } from "../lib/api-types";

export type SortValue = "recent" | "title" | "year" | "rating";

export type CatalogFilters = {
  genre: number | null;
  year: number | null;
  quality: string | null;
  sort: SortValue;
};

export const EMPTY_FILTERS: CatalogFilters = { genre: null, year: null, quality: null, sort: "recent" };

const SORT_OPTIONS: { value: SortValue; label: string }[] = [
  { value: "recent", label: "Recientes" },
  { value: "title", label: "Título (A-Z)" },
  { value: "year", label: "Año" },
  { value: "rating", label: "Mejor valoradas" },
];

/// Etiquetas de calidad más habituales en un panel Xtream — no hay un catálogo cerrado
/// de valores posibles (lo pone el proveedor en el nombre crudo), así que esto es una
/// lista de las más frecuentes, no exhaustiva.
const QUALITY_OPTIONS = ["4K", "FHD", "HD", "SD"];

/** Barra de filtros compartida entre Películas y Series — el backend ya aceptaba estos
 * parámetros desde F4, pero no había ningún control en la interfaz que los usara. */
export function CatalogFilterBar({
  kind,
  filters,
  onChange,
  showQuality,
}: {
  kind: "movie" | "tv";
  filters: CatalogFilters;
  onChange: (filters: CatalogFilters) => void;
  showQuality?: boolean;
}) {
  const genresQuery = useQuery({
    queryKey: ["genres", kind],
    queryFn: () => apiGet<GenreSummary[]>(`/api/genres?kind=${kind}`),
    staleTime: Infinity, // apenas cambian; una recarga completa de la app ya los refresca.
  });

  return (
    <div className="flex flex-wrap items-center gap-3 px-6 sm:px-10">
      <select
        aria-label="Género"
        className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
        value={filters.genre ?? ""}
        onChange={(e) => onChange({ ...filters, genre: e.target.value ? Number(e.target.value) : null })}
      >
        <option value="">Todos los géneros</option>
        {genresQuery.data?.map((g) => (
          <option key={g.id} value={g.id}>
            {g.name}
          </option>
        ))}
      </select>

      <input
        type="number"
        inputMode="numeric"
        placeholder="Año"
        aria-label="Año"
        className="w-24 rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
        value={filters.year ?? ""}
        onChange={(e) => onChange({ ...filters, year: e.target.value ? Number(e.target.value) : null })}
      />

      {showQuality && (
        <select
          aria-label="Calidad"
          className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
          value={filters.quality ?? ""}
          onChange={(e) => onChange({ ...filters, quality: e.target.value || null })}
        >
          <option value="">Cualquier calidad</option>
          {QUALITY_OPTIONS.map((q) => (
            <option key={q} value={q}>
              {q}
            </option>
          ))}
        </select>
      )}

      <select
        aria-label="Ordenar por"
        className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none"
        value={filters.sort}
        onChange={(e) => onChange({ ...filters, sort: e.target.value as SortValue })}
      >
        {SORT_OPTIONS.map((s) => (
          <option key={s.value} value={s.value}>
            {s.label}
          </option>
        ))}
      </select>

      {(filters.genre != null || filters.year != null || filters.quality != null || filters.sort !== "recent") && (
        <button
          type="button"
          onClick={() => onChange(EMPTY_FILTERS)}
          className="text-sm text-neutral-500 underline decoration-dotted hover:text-neutral-300"
        >
          Quitar filtros
        </button>
      )}
    </div>
  );
}
