import { Outlet, createRootRoute } from "@tanstack/react-router";
import { AppLayout } from "../components/AppLayout";
import { AccountGate } from "../features/account/AccountGate";
import { ActiveProfileGate } from "../features/profiles/ActiveProfileGate";
import { PlayerHost } from "../features/player/PlayerHost";
import { PlayerSessionProvider } from "../lib/playerSession";

export const rootRoute = createRootRoute({
  component: () => (
    <AccountGate>
      <ActiveProfileGate>
        <PlayerSessionProvider>
          <AppLayout>
            <Outlet />
          </AppLayout>
          {/* Fuera del layout: el reproductor sigue en pequeño al cambiar de pantalla. */}
          <PlayerHost />
        </PlayerSessionProvider>
      </ActiveProfileGate>
    </AccountGate>
  ),
});
