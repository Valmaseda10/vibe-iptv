import { Outlet, createRootRoute } from "@tanstack/react-router";
import { AppLayout } from "../components/AppLayout";
import { AccountGate } from "../features/account/AccountGate";
import { ActiveProfileGate } from "../features/profiles/ActiveProfileGate";
import { PlayerHost } from "../features/player/PlayerHost";
import { PlayerSessionProvider } from "../lib/playerSession";
import { RatePrompt } from "../components/RatePrompt";
import { UpdateToast } from "../components/UpdateToast";

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
          {/* «¿Te ha gustado?» al terminar algo. */}
          <RatePrompt />
          <UpdateToast />
        </PlayerSessionProvider>
      </ActiveProfileGate>
    </AccountGate>
  ),
});
