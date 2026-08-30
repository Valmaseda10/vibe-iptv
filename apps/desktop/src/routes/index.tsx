import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { HomeScreen } from "../features/home/HomeScreen";

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomeScreen,
});
