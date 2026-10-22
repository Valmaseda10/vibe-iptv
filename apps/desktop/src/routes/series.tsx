import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SeriesScreen } from "../features/series/SeriesScreen";

export const seriesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/series",
  // `view=all`: todo el catálogo con filtros en vez de la portada y las filas.
  validateSearch: (search: Record<string, unknown>): { view?: "all" } => ({ view: search.view === "all" ? "all" : undefined }),
  component: SeriesScreen,
});
