import { useEffect, useRef } from "react";
import { useReminders } from "../lib/reminders";
import { useSportsEvents } from "../lib/sports";
import { canScheduleNotifications, markShownOnce, noticeId, replaceScheduledNotifications, showNotification } from "../lib/notifications";
import type { ScheduledNotice } from "../lib/api-types";

/** Con cuánta antelación avisa un recordatorio (como el aviso de dentro de la app). */
const REMINDER_BEFORE_S = 60;
/** Y un partido de «Mis equipos», que no se ha marcado a mano. */
const MATCH_BEFORE_S = 5 * 60;
/** El PC mira cada tanto si toca algún aviso. */
const CHECK_EVERY_MS = 15_000;

/** Los avisos del sistema del perfil (F17): sus recordatorios y los partidos de lo que
 * sigue. En el iPhone se dejan programados (avisan con la app cerrada) cada vez que cambia
 * la lista; en el PC se enseñan al llegar la hora, una vez cada uno. No pinta nada. */
export function NotificationScheduler() {
  const reminders = useReminders();
  const sports = useSportsEvents(7);

  const notices: ScheduledNotice[] = [];
  for (const r of reminders.list) {
    notices.push({
      id: noticeId(`reminder:${r.channel.id}:${r.start_ts}`),
      title: r.title,
      body: `Empieza en ${r.channel.display_name}`,
      at: r.start_ts - REMINDER_BEFORE_S,
    });
  }
  const reminded = new Set(reminders.list.map((r) => `${r.start_ts}:${r.title}`));
  for (const event of sports.data ?? []) {
    // Las repeticiones (diferido) no avisan: solo lo que se juega de verdad.
    if (!event.followed || event.status === "replay" || event.status === "finished" || reminded.has(`${event.start_ts}:${event.name}`)) continue;
    notices.push({
      id: noticeId(`match:${event.key}`),
      title: event.name,
      body: `${event.competition ? `${event.competition} · ` : ""}Empieza en 5 minutos${event.channels[0] ? ` en ${event.channels[0].name}` : ""}`,
      at: event.start_ts - MATCH_BEFORE_S,
    });
  }
  const signature = notices.map((n) => `${n.id}@${n.at}`).sort().join(",");

  // iPhone: la lista entera, programada, cada vez que cambia.
  const lastScheduled = useRef<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    void canScheduleNotifications().then((can) => {
      if (cancelled || !can || lastScheduled.current === signature) return;
      lastScheduled.current = signature;
      void replaceScheduledNotifications(notices);
    });
    return () => {
      cancelled = true;
    };
    // `signature` resume `notices`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature]);

  // PC: al llegar la hora, una vez.
  const latest = useRef(notices);
  latest.current = notices;
  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    void canScheduleNotifications().then((can) => {
      if (cancelled || can) return;
      const check = () => {
        const now = Date.now() / 1000;
        for (const notice of latest.current) {
          // Hasta diez minutos tarde (la app estaba cerrada o el PC dormido) aún sirve.
          if (notice.at <= now && now - notice.at < 600 && markShownOnce(String(notice.id))) {
            void showNotification(notice.title, notice.body);
          }
        }
      };
      check();
      timer = window.setInterval(check, CHECK_EVERY_MS);
    });
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  return null;
}
