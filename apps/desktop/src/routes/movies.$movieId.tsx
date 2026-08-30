import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { MovieDetailScreen } from "../features/movies/MovieDetailScreen";

export const movieDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/movies/$movieId",
  component: MovieDetailScreen,
});
