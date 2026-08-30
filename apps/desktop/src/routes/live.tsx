import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { LiveScreen } from "../features/live/LiveScreen";

export const liveRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/live",
  component: LiveScreen,
});
