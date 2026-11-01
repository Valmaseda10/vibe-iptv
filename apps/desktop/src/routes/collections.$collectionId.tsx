import { createRoute } from "@tanstack/react-router";
import { rootRoute } from "./__root";
import { CollectionScreen } from "../features/collections/CollectionScreen";

export const collectionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/collections/$collectionId",
  component: CollectionScreen,
});
