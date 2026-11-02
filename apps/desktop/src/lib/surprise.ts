import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { apiGetOrNull } from "./api-client";
import { useActiveProfile } from "./activeProfile";
import type { RandomTitleResponse } from "./api-types";

/** «Sorpréndeme»: lleva a la ficha de un título al azar, nunca abre el reproductor solo
 * (sigue siendo el usuario quien decide reproducir, R1). En la barra lateral del PC y en
 * «Mi lista» del iPhone. */
export function useSurpriseMe() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const [surprising, setSurprising] = useState(false);
  async function surpriseMe() {
    if (surprising || profileId === null) return;
    setSurprising(true);
    try {
      const pick = await apiGetOrNull<RandomTitleResponse>(`/api/random?profile_id=${profileId}`);
      if (!pick) return;
      if (pick.kind === "series") navigate({ to: "/series/$seriesId", params: { seriesId: String(pick.id) } });
      else navigate({ to: "/movies/$movieId", params: { movieId: String(pick.id) } });
    } finally {
      setSurprising(false);
    }
  }
  return { surpriseMe, surprising };
}
