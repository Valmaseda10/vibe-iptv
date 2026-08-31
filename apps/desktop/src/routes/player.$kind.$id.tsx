import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { PlayerScreen } from "../features/player/PlayerScreen";

type PlayerSearch = {
  seriesId?: string;
  categoryId?: string;
};

export const playerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/player/$kind/$id",
  validateSearch: (search: Record<string, unknown>): PlayerSearch => ({
    seriesId: typeof search.seriesId === "string" ? search.seriesId : undefined,
    categoryId: typeof search.categoryId === "string" ? search.categoryId : undefined,
  }),
  component: PlayerScreen,
});
