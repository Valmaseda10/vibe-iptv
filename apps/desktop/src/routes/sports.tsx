import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SportsScreen } from "../features/sports/SportsScreen";

export const sportsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sports",
  component: SportsScreen,
});
