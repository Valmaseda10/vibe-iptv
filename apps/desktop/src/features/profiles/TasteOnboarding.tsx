import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { BrowseItem, TasteCandidatesResponse } from "../../lib/api-types";
import { TastePicker } from "./TastePicker";

/** Con menos títulos no merece la pena preguntar (catálogo sin emparejar con TMDB). */
const MIN_CANDIDATES = 9;

function dismissedKey(profileId: number): string {
  return `vibe.tastePicker.dismissed.${profileId}`;
}

function wasDismissed(profileId: number): boolean {
  try {
    return window.localStorage.getItem(dismissedKey(profileId)) === "1";
  } catch {
    return false;
  }
}

/** Abre «Elige lo que te gusta» al entrar en Inicio si el perfil aún no tiene gustos (lo
 * decide el núcleo: `GET /api/taste/candidates`). Saltarlo se recuerda en este aparato;
 * elegir guarda «Me gusta» en cada título, que viaja a los demás aparatos por la nube. */
export function TasteOnboarding() {
  const queryClient = useQueryClient();
  const { profileId } = useActiveProfile();
  const [closedFor, setClosedFor] = useState<Set<number>>(new Set());
  const [saving, setSaving] = useState(false);
  const closed = profileId === null || closedFor.has(profileId) || wasDismissed(profileId);

  const query = useQuery({
    queryKey: ["taste-candidates", profileId],
    queryFn: () => apiGet<TasteCandidatesResponse>(`/api/taste/candidates?profile_id=${profileId}&seed=${Math.floor(Math.random() * 2 ** 32)}`),
    enabled: !closed,
    staleTime: Infinity,
  });

  if (closed || !query.data?.needed || query.data.items.length < MIN_CANDIDATES) return null;

  const close = (remember: boolean) => {
    if (profileId === null) return;
    if (remember) {
      try {
        window.localStorage.setItem(dismissedKey(profileId), "1");
      } catch {
        // Sin almacenamiento: volverá a preguntar la próxima vez.
      }
    }
    setClosedFor((current) => new Set(current).add(profileId));
  };

  async function save(picked: BrowseItem[]) {
    if (profileId === null) return;
    setSaving(true);
    try {
      await Promise.all(picked.map((item) => apiPost("/api/ratings", { profile_id: profileId, item_kind: item.kind, item_id: item.id, rating: 1 })));
    } catch {
      // Lo que se haya guardado, guardado queda; el resto se puede valorar desde la ficha.
    }
    setSaving(false);
    close(true);
    // Las filas se vuelven a pedir y esperan a las recomendaciones nuevas (`pending`).
    for (const key of ["browse", "ratings", "taste-candidates", "home"]) queryClient.invalidateQueries({ queryKey: [key] });
  }

  return <TastePicker items={query.data.items} saving={saving} onDone={(picked) => void save(picked)} onSkip={() => close(true)} />;
}
