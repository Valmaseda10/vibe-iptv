import { Link } from "@tanstack/react-router";
import { ChevronIcon } from "../../components/icons";
import { ProfileAvatar, useActiveProfileSummary } from "../../components/ProfileAvatar";

/** Arriba de la lista: el perfil activo y su cuenta, como «Apple ID» en los Ajustes de iOS. */
export function SettingsProfileHeader() {
  const profile = useActiveProfileSummary();
  return (
    <Link
      to="/settings/$section"
      params={{ section: "cuenta" }}
      className="flex items-center gap-3 rounded-xl bg-white/[0.04] p-3 outline-none ring-1 ring-white/[0.06] transition-colors hover:bg-white/[0.07] focus-visible:ring-white/70 desk:hidden ios:rounded-[18px] ios:bg-ios-surface-1 ios:ring-0"
    >
      <ProfileAvatar className="h-12 w-12 text-2xl" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[17px] font-semibold text-white">{profile?.name ?? ""}</span>
        <span className="text-[13px] text-neutral-400 ios:text-ios-label-2">Cuenta, perfiles y contraseña</span>
      </span>
      <ChevronIcon className="h-4 w-4 text-neutral-600" />
    </Link>
  );
}
