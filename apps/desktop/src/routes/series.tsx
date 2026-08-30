import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SeriesScreen } from "../features/series/SeriesScreen";

export const seriesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/series",
  component: SeriesScreen,
});
