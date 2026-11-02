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
import { settingsSectionRoute } from "./routes/settings.$section";
import { playerRoute } from "./routes/player.$kind.$id";
import { collectionRoute } from "./routes/collections.$collectionId";
import { personRoute } from "./routes/people.$personId";
import { sportsRoute } from "./routes/sports";
import { yearRoute } from "./routes/year";

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
  settingsSectionRoute,
  playerRoute,
  collectionRoute,
  personRoute,
  sportsRoute,
  yearRoute,
]);

/** Transiciones entre pantallas (F17): un fundido corto y, al abrir una ficha desde su
 * portada, la portada crece hasta el fondo de la ficha («vibe-hero»). Nunca al entrar,
 * salir o cambiar de canal en el reproductor (el zapping tiene que ser instantáneo), ni si
 * solo cambian los filtros de una pantalla. Solo donde la WebView sabe de «tipos» de
 * transición: sin ellos, el enrutador las haría siempre, también en el zapping. */
const canTypeTransitions = typeof window !== "undefined" && window.CSS?.supports?.("selector(:active-view-transition-type(a))") === true;

export const router = createRouter({
  routeTree,
  defaultViewTransition: canTypeTransitions
    ? {
        types: ({ fromLocation, toLocation, pathChanged }) => {
          if (!pathChanged || !fromLocation) return false;
          if (fromLocation.pathname.startsWith("/player") || toLocation.pathname.startsWith("/player")) return false;
          return ["page"];
        },
      }
    : false,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
