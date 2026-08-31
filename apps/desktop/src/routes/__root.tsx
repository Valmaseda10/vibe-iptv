import { Outlet, createRootRoute } from "@tanstack/react-router";
import { AppLayout } from "../components/AppLayout";
import { AccountGate } from "../features/account/AccountGate";
import { ActiveProfileGate } from "../features/profiles/ActiveProfileGate";

export const rootRoute = createRootRoute({
  component: () => (
    <AccountGate>
      <ActiveProfileGate>
        <AppLayout>
          <Outlet />
        </AppLayout>
      </ActiveProfileGate>
    </AccountGate>
  ),
});
