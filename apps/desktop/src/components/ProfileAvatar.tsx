import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../lib/api-client";
import { useActiveProfile } from "../lib/activeProfile";
import { parseAvatar } from "../lib/avatar";
import type { ProfileSummary } from "../lib/api-types";

/** El perfil activo (nombre y avatar), de la misma consulta que el resto de la app. */
export function useActiveProfileSummary(): ProfileSummary | null {
  const { profileId } = useActiveProfile();
  const query = useQuery({ queryKey: ["profiles"], queryFn: () => apiGet<ProfileSummary[]>("/api/profiles") });
  return query.data?.find((p) => p.id === profileId) ?? null;
}

/** Avatar del perfil activo: su icono sobre su color (o la inicial). */
export function ProfileAvatar({ className = "h-6 w-6 text-xs" }: { className?: string }) {
  const profile = useActiveProfileSummary();
  if (!profile) return <span className={`inline-block rounded-md bg-neutral-700 ${className}`} aria-hidden />;
  const { icon, color } = parseAvatar(profile);
  return (
    <span aria-hidden className={`inline-flex shrink-0 items-center justify-center rounded-md font-semibold text-white ${className}`} style={{ backgroundColor: color }}>
      {icon ?? profile.name.charAt(0).toUpperCase()}
    </span>
  );
}
