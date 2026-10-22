import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { MoviesScreen } from "../features/movies/MoviesScreen";

export const moviesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/movies",
  // `view=all`: todo el catálogo con filtros en vez de la portada y las filas.
  validateSearch: (search: Record<string, unknown>): { view?: "all" } => ({ view: search.view === "all" ? "all" : undefined }),
  component: MoviesScreen,
});
