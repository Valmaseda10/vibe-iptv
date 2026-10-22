import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost } from "./api-client";
import { useActiveProfile } from "./activeProfile";
import type { RatingItem } from "./api-types";

/** Valoración: -1 «No es para mí», 1 «Me gusta», 2 «Me encanta». */
export type RatingValue = -1 | 1 | 2;

export const RATING_OPTIONS: { value: RatingValue; icon: string; label: string }[] = [
  { value: -1, icon: "👎", label: "No es para mí" },
  { value: 1, icon: "👍", label: "Me gusta" },
  { value: 2, icon: "👍👍", label: "Me encanta" },
];

/** Valoraciones del perfil activo, con actualización optimista (como los favoritos). Al
 * cambiar una, Inicio se pone al día: las recomendaciones dependen de ellas. */
export function useRatings() {
  const queryClient = useQueryClient();
  const { profileId } = useActiveProfile();
  const queryKey = ["ratings", profileId];

  const query = useQuery({
    queryKey,
    queryFn: () => apiGet<RatingItem[]>(`/api/ratings?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const ratingOf = (itemKind: string, itemId: number): RatingValue | null => {
    const found = (query.data ?? []).find((r) => r.item_kind === itemKind && r.item_id === itemId);
    return found ? (found.rating as RatingValue) : null;
  };

  const mutation = useMutation({
    mutationFn: async ({ itemKind, itemId, rating }: { itemKind: string; itemId: number; rating: RatingValue | null }) => {
      if (rating === null) await apiDelete(`/api/ratings?profile_id=${profileId}&item_kind=${itemKind}&item_id=${itemId}`);
      else await apiPost("/api/ratings", { profile_id: profileId, item_kind: itemKind, item_id: itemId, rating });
    },
    onMutate: async ({ itemKind, itemId, rating }) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<RatingItem[]>(queryKey);
      queryClient.setQueryData<RatingItem[]>(queryKey, (old) => {
        const rest = (old ?? []).filter((r) => !(r.item_kind === itemKind && r.item_id === itemId));
        return rating === null ? rest : [{ item_kind: itemKind, item_id: itemId, rating }, ...rest];
      });
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) queryClient.setQueryData(queryKey, context.previous);
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey });
      queryClient.invalidateQueries({ queryKey: ["home"] });
    },
  });

  return {
    ratingOf,
    setRating: (itemKind: string, itemId: number, rating: RatingValue | null) => mutation.mutate({ itemKind, itemId, rating }),
  };
}
