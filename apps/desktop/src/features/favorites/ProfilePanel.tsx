import { Link } from "@tanstack/react-router";
import { useActiveProfile } from "../../lib/activeProfile";
import { useSurpriseMe } from "../../lib/surprise";
import { ProfileAvatar, useActiveProfileSummary } from "../../components/ProfileAvatar";
import { DiceIcon, SettingsIcon, TrophyIcon } from "../../components/icons";

/** Arriba de «Mi lista» en el iPhone (2026-11-02): el perfil, Ajustes, «Tu año»,
 * «Sorpréndeme» y «Cambiar de perfil», como «Mi Netflix». En el PC todo eso está en la
 * barra lateral. */
export function ProfilePanel() {
  const profile = useActiveProfileSummary();
  const { setProfileId } = useActiveProfile();
  const { surpriseMe, surprising } = useSurpriseMe();
  const tile =
    "flex flex-col items-center justify-center gap-1.5 rounded-2xl bg-white/[0.07] px-2 py-3 text-xs font-medium text-white outline-none active:bg-white/15 focus-visible:ring-2 focus-visible:ring-white/70";

  return (
    <section className="flex flex-col gap-4 desk:hidden" aria-label="Tu perfil">
      <div className="flex items-center gap-3">
        <ProfileAvatar className="h-14 w-14 text-2xl" />
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-xl font-bold text-white">{profile?.name ?? ""}</span>
          <button type="button" onClick={() => setProfileId(null)} className="w-fit text-sm text-white/60 outline-none active:text-white">
            Cambiar de perfil
          </button>
        </div>
      </div>
      <div className="grid grid-cols-3 gap-2">
        <Link to="/settings" className={tile}>
          <SettingsIcon className="h-6 w-6" />
          Ajustes
        </Link>
        <Link to="/year" className={tile}>
          <TrophyIcon className="h-6 w-6" />
          Tu año
        </Link>
        <button type="button" onClick={() => void surpriseMe()} disabled={surprising} className={`${tile} disabled:opacity-50`}>
          <DiceIcon className={`h-6 w-6 ${surprising ? "animate-spin" : ""}`} />
          Sorpréndeme
        </button>
      </div>
    </section>
  );
}
