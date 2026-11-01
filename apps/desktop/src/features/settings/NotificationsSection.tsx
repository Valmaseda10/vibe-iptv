import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useActiveProfile } from "../../lib/activeProfile";
import { ensureNotificationPermission, notificationPermission, type NotificationPermission } from "../../lib/notifications";
import { IS_IOS } from "../../lib/platform";

/** Ajustes → Avisos (F17): los avisos de episodios nuevos de las series que sigue el
 * perfil, a mano y en cada aparato (apagados de serie). Los recordatorios y los partidos de
 * «Mis equipos» avisan siempre: se piden al marcarlos. */
export function NotificationsSection() {
  const { profileId } = useActiveProfile();
  const [newEpisodes, setNewEpisodes] = useState<boolean | null>(null);
  const [permission, setPermission] = useState<NotificationPermission | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (profileId === null) return;
    let cancelled = false;
    invoke<boolean>("new_episode_alerts", { profileId })
      .then((on) => !cancelled && setNewEpisodes(on))
      .catch(() => !cancelled && setNewEpisodes(false));
    void notificationPermission().then((p) => !cancelled && setPermission(p));
    return () => {
      cancelled = true;
    };
  }, [profileId]);

  async function toggle() {
    if (profileId === null || newEpisodes === null) return;
    setSaving(true);
    const next = !newEpisodes;
    try {
      if (next) setPermission((await ensureNotificationPermission()) ? "granted" : "denied");
      await invoke("set_new_episode_alerts", { profileId, enabled: next });
      setNewEpisodes(next);
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-4 sm:p-6 ios:mb-8 ios:rounded-[22px] ios:border-0 ios:bg-ios-surface-1 ios:p-5">
      <h2 className="mb-4 text-xl font-medium ios:text-[22px] ios:font-bold ios:tracking-tight">Avisos</h2>
      <button
        type="button"
        role="switch"
        aria-checked={newEpisodes === true}
        disabled={newEpisodes === null || saving}
        onClick={() => void toggle()}
        className="flex w-full items-center gap-4 rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-white/70 disabled:opacity-60"
      >
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="font-medium text-white">Episodios nuevos de mis series</span>
          <span className="text-sm text-neutral-400 ios:text-[15px] ios:text-ios-label-2">
            De las series de Mi lista y las que has visto estos meses. Solo en este {IS_IOS ? "aparato" : "PC"} y para este perfil.
            {IS_IOS ? " iOS decide cuándo dejar mirar con la app cerrada: puede tardar unas horas." : " Avisa con la app abierta."}
          </span>
        </span>
        <span className={`relative h-7 w-12 shrink-0 rounded-full transition-colors ${newEpisodes ? "bg-red-600 ios:bg-[#34c759]" : "bg-neutral-700"}`}>
          <span className={`absolute top-0.5 h-6 w-6 rounded-full bg-white shadow transition-transform ${newEpisodes ? "translate-x-5" : "translate-x-0.5"}`} />
        </span>
      </button>
      {permission === "denied" && (
        <p className="mt-3 text-sm text-amber-400">
          {IS_IOS ? "Los avisos están desactivados para VIBE IPTV en Ajustes del iPhone → Notificaciones." : "Windows tiene los avisos de VIBE IPTV desactivados (Configuración → Sistema → Notificaciones)."}
        </p>
      )}
      <p className="mt-4 text-sm text-neutral-500 ios:text-[15px]">Los recordatorios de la guía y los partidos de «Mis equipos» (en Deportes) avisan siempre.</p>
    </section>
  );
}
