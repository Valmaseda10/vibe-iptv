import { useNavigate, useSearch } from "@tanstack/react-router";
import { BrowseScreen } from "../browse/BrowseScreen";
import { SeriesCatalog } from "./SeriesCatalog";

/** Series: portada y filas como en Netflix y, con «Catálogo completo», todo en cuadrícula
 * con filtros (`?view=all`). */
export function SeriesScreen() {
  const navigate = useNavigate();
  const { view } = useSearch({ from: "/series" });
  if (view === "all") return <SeriesCatalog />;
  return (
    <BrowseScreen
      page="series"
      title="Series"
      actions={
        <button
          type="button"
          onClick={() => navigate({ to: "/series", search: { view: "all" } })}
          className="rounded-full border border-white/40 bg-black/40 px-4 py-1.5 text-sm font-medium text-white outline-none backdrop-blur-sm transition hover:bg-white/15 focus-visible:ring-2 focus-visible:ring-red-600"
        >
          Catálogo completo ›
        </button>
      }
    />
  );
}
