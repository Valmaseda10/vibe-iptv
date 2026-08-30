import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SeriesDetailScreen } from "../features/series/SeriesDetailScreen";

export const seriesDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/series/$seriesId",
  component: SeriesDetailScreen,
});
