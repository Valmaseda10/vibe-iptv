import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { NativePlayerScreen } from "../features/player/NativePlayerScreen";
import { IS_IOS } from "../lib/platform";
import type { PlayerSearch } from "../lib/playerSession";

export const playerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/player/$kind/$id",
  validateSearch: (search: Record<string, unknown>): PlayerSearch => ({
    seriesId: typeof search.seriesId === "string" ? search.seriesId : undefined,
    categoryId: typeof search.categoryId === "string" ? search.categoryId : undefined,
    catchupStart: typeof search.catchupStart === "number" ? search.catchupStart : undefined,
    catchupDuration: typeof search.catchupDuration === "number" ? search.catchupDuration : undefined,
    source: typeof search.source === "number" ? search.source : undefined,
    audioTrack: typeof search.audioTrack === "number" ? search.audioTrack : undefined,
    favorites: search.favorites === true ? true : undefined,
  }),
  // En iPhone y iPad reproduce VLC en una pantalla nativa (F15). En el PC la ruta no pinta
  // nada: el reproductor lo monta `PlayerHost` por encima de toda la app, para que al
  // volver atrás siga reproduciéndose en pequeño.
  component: IS_IOS ? NativePlayerScreen : () => null,
});
