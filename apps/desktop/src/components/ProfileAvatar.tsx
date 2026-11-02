import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../lib/api-client";
import { useActiveProfile } from "../lib/activeProfile";
import { parseAvatar } from "../lib/avatar";
import { AvatarImage } from "./AvatarImage";
import type { ProfileSummary } from "../lib/api-types";

/** El perfil activo (nombre y avatar), de la misma consulta que el resto de la app. */
export function useActiveProfileSummary(): ProfileSummary | null {
  const { profileId } = useActiveProfile();
  const query = useQuery({ queryKey: ["profiles"], queryFn: () => apiGet<ProfileSummary[]>("/api/profiles") });
  return query.data?.find((p) => p.id === profileId) ?? null;
}

/** Avatar del perfil activo (icono, personaje, imagen de una película...). */
export function ProfileAvatar({ className = "h-6 w-6 text-xs" }: { className?: string }) {
  const profile = useActiveProfileSummary();
  if (!profile) return <span className={`inline-block rounded-md bg-neutral-700 ${className}`} aria-hidden />;
  return <AvatarImage avatar={parseAvatar(profile)} name={profile.name} className={`rounded-md ${className}`} />;
}
