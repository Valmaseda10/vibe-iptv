import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { MoviesScreen } from "../features/movies/MoviesScreen";

export const moviesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/movies",
  component: MoviesScreen,
});
