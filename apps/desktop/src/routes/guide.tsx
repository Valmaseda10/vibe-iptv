import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { GuideScreen } from "../features/guide/GuideScreen";

export const guideRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/guide",
  component: GuideScreen,
});
