import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SearchScreen } from "../features/search/SearchScreen";

export const searchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/search",
  component: SearchScreen,
});
