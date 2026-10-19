import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useReminders } from "../lib/reminders";
import { BellIcon } from "./icons";

/** Con cuánta antelación avisa un recordatorio. */
const NOTICE_BEFORE_SECONDS = 60;

/** Aviso de "empieza tu programa" (F13), en cualquier pantalla de la app. Un botón lleva
 * al canal; cerrar el aviso borra el recordatorio. */
export function ReminderNotifier() {
  const navigate = useNavigate();
  const reminders = useReminders();
  const [now, setNow] = useState(() => Date.now() / 1000);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now() / 1000), 5_000);
    return () => window.clearInterval(timer);
  }, []);

  const due = reminders.list.filter((r) => r.start_ts - NOTICE_BEFORE_SECONDS <= now && r.stop_ts > now);
  if (due.length === 0) return null;

  return (
    <div className="fixed bottom-6 right-6 z-50 flex w-80 flex-col gap-3" role="status" aria-live="polite">
      {due.map((r) => (
        <div key={`${r.channel.id}-${r.start_ts}`} className="rounded-lg border border-amber-500/40 bg-neutral-900/95 p-4 shadow-xl">
          <p className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-amber-400">
            <BellIcon className="h-3.5 w-3.5" />
            {r.start_ts > now ? "Empieza en un momento" : "Ya ha empezado"}
          </p>
          <p className="mt-1 truncate font-medium text-white">{r.title}</p>
          <p className="truncate text-sm text-neutral-400">{r.channel.display_name}</p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              onClick={() => {
                reminders.dismiss(r);
                navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(r.channel.id) }, search: {} });
              }}
              className="rounded bg-red-600 px-3 py-1.5 text-sm font-medium outline-none hover:bg-red-500 focus-visible:ring-2 focus-visible:ring-white/70"
            >
              Ver ahora
            </button>
            <button
              type="button"
              onClick={() => reminders.dismiss(r)}
              className="rounded bg-neutral-800 px-3 py-1.5 text-sm outline-none hover:bg-neutral-700 focus-visible:ring-2 focus-visible:ring-white/70"
            >
              Cerrar
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
