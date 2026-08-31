import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { FavoritesScreen } from "../features/favorites/FavoritesScreen";

export const favoritesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/favorites",
  component: FavoritesScreen,
});
