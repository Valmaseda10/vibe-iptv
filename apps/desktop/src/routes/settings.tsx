import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SettingsScreen } from "../features/settings/SettingsScreen";

export const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsScreen,
});
