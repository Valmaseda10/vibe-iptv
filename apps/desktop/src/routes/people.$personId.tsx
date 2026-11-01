import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { PersonScreen } from "../features/people/PersonScreen";

export const personRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/people/$personId",
  component: PersonScreen,
});
