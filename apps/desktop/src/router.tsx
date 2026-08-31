import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "./routes/__root";
import { indexRoute } from "./routes/index";
import { liveRoute } from "./routes/live";
import { moviesRoute } from "./routes/movies";
import { movieDetailRoute } from "./routes/movies.$movieId";
import { seriesRoute } from "./routes/series";
import { seriesDetailRoute } from "./routes/series.$seriesId";
import { searchRoute } from "./routes/search";
import { settingsRoute } from "./routes/settings";
import { playerRoute } from "./routes/player.$kind.$id";

const routeTree = rootRoute.addChildren([
  indexRoute,
  liveRoute,
  moviesRoute,
  movieDetailRoute,
  seriesRoute,
  seriesDetailRoute,
  searchRoute,
  settingsRoute,
  playerRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
