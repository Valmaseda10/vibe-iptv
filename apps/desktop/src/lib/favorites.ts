import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost } from "./api-client";
import type { FavoriteItem } from "./api-types";

/** Favoritos (F8): una sola lista para toda la app, con actualización optimista — el
 * conmutador de una tarjeta cambia al instante, sin esperar la respuesta del servidor. */
export function useFavorites() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["favorites"],
    queryFn: () => apiGet<FavoriteItem[]>("/api/favorites"),
  });

  const isFavorite = (itemKind: string, itemId: number) =>
    (query.data ?? []).some((f) => f.item_kind === itemKind && f.item_id === itemId);

  const mutation = useMutation({
    mutationFn: async ({ itemKind, itemId, next }: { itemKind: string; itemId: number; next: boolean }) => {
      if (next) await apiPost("/api/favorites", { item_kind: itemKind, item_id: itemId });
      else await apiDelete(`/api/favorites?item_kind=${itemKind}&item_id=${itemId}`);
    },
    onMutate: async ({ itemKind, itemId, next }) => {
      await queryClient.cancelQueries({ queryKey: ["favorites"] });
      const previous = queryClient.getQueryData<FavoriteItem[]>(["favorites"]);
      queryClient.setQueryData<FavoriteItem[]>(["favorites"], (old) => {
        const list = old ?? [];
        // Título/portada reales llegan al `invalidateQueries` de `onSettled`, un
        // instante después; el marcador optimista no los necesita porque nada lo
        // pinta antes de que eso ocurra (isFavorite() solo mira kind+id).
        return next
          ? [...list, { item_kind: itemKind, item_id: itemId, title: "", poster_path: null }]
          : list.filter((f) => !(f.item_kind === itemKind && f.item_id === itemId));
      });
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) queryClient.setQueryData(["favorites"], context.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["favorites"] }),
  });

  return {
    isFavorite,
    toggleFavorite: (itemKind: string, itemId: number, next: boolean) => mutation.mutate({ itemKind, itemId, next }),
  };
}
