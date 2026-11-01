import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "./routes/__root";
import { indexRoute } from "./routes/index";
import { liveRoute } from "./routes/live";
import { guideRoute } from "./routes/guide";
import { moviesRoute } from "./routes/movies";
import { movieDetailRoute } from "./routes/movies.$movieId";
import { seriesRoute } from "./routes/series";
import { seriesDetailRoute } from "./routes/series.$seriesId";
import { searchRoute } from "./routes/search";
import { favoritesRoute } from "./routes/favorites";
import { settingsRoute } from "./routes/settings";
import { playerRoute } from "./routes/player.$kind.$id";
import { collectionRoute } from "./routes/collections.$collectionId";
import { personRoute } from "./routes/people.$personId";

const routeTree = rootRoute.addChildren([
  indexRoute,
  liveRoute,
  guideRoute,
  moviesRoute,
  movieDetailRoute,
  seriesRoute,
  seriesDetailRoute,
  searchRoute,
  favoritesRoute,
  settingsRoute,
  playerRoute,
  collectionRoute,
  personRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
