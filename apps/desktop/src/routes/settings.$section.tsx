import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { SettingsScreen } from "../features/settings/SettingsScreen";

function SettingsSectionScreen() {
  const { section } = settingsSectionRoute.useParams();
  return <SettingsScreen section={section} />;
}

/** Una sección de Ajustes (en el iPhone, su propia pantalla). */
export const settingsSectionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/$section",
  component: SettingsSectionScreen,
});
