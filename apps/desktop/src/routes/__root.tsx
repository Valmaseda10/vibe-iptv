import { Outlet, createRootRoute } from "@tanstack/react-router";
import { AppLayout } from "../components/AppLayout";
import { ActiveProfileGate } from "../features/profiles/ActiveProfileGate";

export const rootRoute = createRootRoute({
  component: () => (
    <ActiveProfileGate>
      <AppLayout>
        <Outlet />
      </AppLayout>
    </ActiveProfileGate>
  ),
});
