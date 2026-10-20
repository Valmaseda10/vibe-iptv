import { useEffect, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { ProfileSummary } from "../../lib/api-types";
import { ProfilesScreen } from "./ProfilesScreen";

/** Envuelve toda la app: mientras no haya un perfil activo elegido para esta sesión (o
 * el que había guardado ya no existe — se borró desde otro sitio), enseña el selector
 * de perfiles en vez de `children`. */
export function ActiveProfileGate({ children }: { children: ReactNode }) {
  const { profileId, setProfileId } = useActiveProfile();

  const profilesQuery = useQuery({
    queryKey: ["profiles"],
    queryFn: () => apiGet<ProfileSummary[]>("/api/profiles"),
  });

  const activeStillExists = profileId !== null && (profilesQuery.data?.some((p) => p.id === profileId) ?? false);

  useEffect(() => {
    // Limpia una referencia a un perfil que ya no existe (se borró desde otro sitio) —
    // en un efecto, no durante el render, para no disparar un `setState` a mitad de
    // pintar el árbol.
    if (profilesQuery.data && profileId !== null && !activeStillExists) {
      setProfileId(null);
    }
  }, [profilesQuery.data, profileId, activeStillExists, setProfileId]);

  if (profilesQuery.isPending) {
    return <div className="flex h-full min-h-screen items-center justify-center bg-neutral-950 text-neutral-500 ios:bg-black">Cargando…</div>;
  }

  if (!activeStillExists) {
    return <ProfilesScreen />;
  }

  return <>{children}</>;
}
