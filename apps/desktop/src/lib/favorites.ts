import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost } from "./api-client";
import { useActiveProfile } from "./activeProfile";
import type { FavoriteItem } from "./api-types";

/** Favoritos: una lista por perfil, con actualización optimista — el conmutador de una
 * tarjeta cambia al instante, sin esperar la respuesta del servidor. */
export function useFavorites() {
  const queryClient = useQueryClient();
  const { profileId } = useActiveProfile();
  const queryKey = ["favorites", profileId];

  const query = useQuery({
    queryKey,
    queryFn: () => apiGet<FavoriteItem[]>(`/api/favorites?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const isFavorite = (itemKind: string, itemId: number) =>
    (query.data ?? []).some((f) => f.item_kind === itemKind && f.item_id === itemId);

  const mutation = useMutation({
    mutationFn: async ({ itemKind, itemId, next }: { itemKind: string; itemId: number; next: boolean }) => {
      if (next) await apiPost("/api/favorites", { profile_id: profileId, item_kind: itemKind, item_id: itemId });
      else await apiDelete(`/api/favorites?profile_id=${profileId}&item_kind=${itemKind}&item_id=${itemId}`);
    },
    onMutate: async ({ itemKind, itemId, next }) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<FavoriteItem[]>(queryKey);
      queryClient.setQueryData<FavoriteItem[]>(queryKey, (old) => {
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
      if (context?.previous) queryClient.setQueryData(queryKey, context.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  });

  return {
    isFavorite,
    toggleFavorite: (itemKind: string, itemId: number, next: boolean) => mutation.mutate({ itemKind, itemId, next }),
  };
}
