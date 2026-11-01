import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { YearReviewScreen } from "../features/year/YearReviewScreen";

export const yearRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/year",
  component: YearReviewScreen,
});
