import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { PlayerScreen } from "../features/player/PlayerScreen";

type PlayerSearch = {
  seriesId?: string;
  categoryId?: string;
  /** Epoch (segundos UTC) de inicio, solo si se abre desde la Guía como catch-up (F9)
   * en vez de directo. Su sola presencia es lo que distingue los dos modos. */
  catchupStart?: number;
  /** Minutos a pedir desde `catchupStart`. Si falta, el backend usa un valor por defecto. */
  catchupDuration?: number;
  /** Id de `movie_sources` elegido en la ficha (F "premium": selector de calidad/fuente)
   * cuando la película tiene más de una — el backend ya lo aceptaba desde F6, sin
   * ningún selector todavía en la interfaz que lo mandara. */
  source?: number;
  /** Índice relativo de pista de audio elegida (F "premium" — selector de pista de
   * audio). Solo tiene efecto en nivel 2 (remux); en nivel 1 no hay selector todavía. */
  audioTrack?: number;
};

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
  }),
  component: PlayerScreen,
});
