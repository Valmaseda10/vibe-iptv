import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useActiveProfile } from "../../lib/activeProfile";
import { ensureNotificationPermission, notificationPermission, type NotificationPermission } from "../../lib/notifications";
import { IS_IOS } from "../../lib/platform";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { Toggle } from "../../components/Toggle";
import { useHomePreference } from "../../lib/sports";

/** Ajustes → Avisos (F17): los de episodios nuevos de las series que sigue el perfil, a
 * mano y en cada aparato (apagados de serie). Los recordatorios y los partidos de «Mis
 * equipos» avisan siempre: se piden al marcarlos. */
export function NotificationsSection() {
  const { profileId } = useActiveProfile();
  const [newEpisodes, setNewEpisodes] = useState<boolean | null>(null);
  const [permission, setPermission] = useState<NotificationPermission | null>(null);
  const [saving, setSaving] = useState(false);
  const sports = useHomePreference();

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

  async function toggle(next: boolean) {
    if (profileId === null) return;
    setSaving(true);
    try {
      if (next) setPermission((await ensureNotificationPermission()) ? "granted" : "denied");
      await invoke("set_new_episode_alerts", { profileId, enabled: next });
      setNewEpisodes(next);
    } finally {
      setSaving(false);
    }
  }

  const device = IS_IOS ? "aparato" : "PC";
  return (
    <div className="flex flex-col gap-6">
      <SettingsCard
        title="Series"
        footer={IS_IOS ? "iOS decide cuándo dejar mirar con la app cerrada: el aviso puede tardar unas horas." : "En el PC avisa con la app abierta."}
      >
        <SettingsRow inline
          title="Episodios nuevos de mis series"
          description={`De las series de Mi lista y de las que has visto estos meses. Solo en este ${device} y para este perfil.`}
        >
          <Toggle checked={newEpisodes === true} onChange={(next) => void toggle(next)} disabled={newEpisodes === null || saving} label="Episodios nuevos de mis series" />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Deportes"
        footer="Seguir un equipo hace que sus partidos salgan destacados en Deportes y, en directo, en la portada de Inicio. Los avisos son aparte. Viaja con tu cuenta."
      >
        <SettingsRow inline title="Partidos de mis equipos" description="Un aviso 5 minutos antes de que jueguen los equipos y competiciones que sigues.">
          <Toggle checked={sports.notify} onChange={sports.setNotify} disabled={!sports.loaded} label="Partidos de mis equipos" />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard title="Siempre activos">
        <SettingsRow title="Recordatorios" description="Lo que marcas con «Avísame» en la guía o en un partido." />
      </SettingsCard>

      {permission === "denied" && (
        <p className="text-sm text-amber-400">
          {IS_IOS ? "Los avisos están desactivados para VIBE IPTV en Ajustes del iPhone → Notificaciones." : "Windows tiene los avisos de VIBE IPTV desactivados (Configuración → Sistema → Notificaciones)."}
        </p>
      )}
    </div>
  );
}
