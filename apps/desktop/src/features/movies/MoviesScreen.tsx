import { useNavigate, useSearch } from "@tanstack/react-router";
import { BrowseScreen } from "../browse/BrowseScreen";
import { MoviesCatalog } from "./MoviesCatalog";

/** Películas: portada y filas como en Netflix y, con «Catálogo completo», todo en cuadrícula
 * con filtros (`?view=all`). */
export function MoviesScreen() {
  const navigate = useNavigate();
  const { view } = useSearch({ from: "/movies" });
  if (view === "all") return <MoviesCatalog />;
  return (
    <BrowseScreen
      page="movies"
      title="Películas"
      actions={
        <button
          type="button"
          onClick={() => navigate({ to: "/movies", search: { view: "all" } })}
          className="rounded-full border border-white/40 bg-black/40 px-4 py-1.5 text-sm font-medium text-white outline-none backdrop-blur-sm transition hover:bg-white/15 focus-visible:ring-2 focus-visible:ring-red-600"
        >
          Catálogo completo ›
        </button>
      }
    />
  );
}
