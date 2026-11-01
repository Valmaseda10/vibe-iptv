import { useMemo, useState } from "react";
import { useReminders } from "../../lib/reminders";
import { SPORT_LABELS, dayLabel, isOnNow, sportEmoji, useSportsEvents } from "../../lib/sports";
import type { SportEventResponse } from "../../lib/api-types";
import { SportEventCard } from "./SportEventCard";
import { FollowTeamsSheet } from "./FollowTeamsSheet";

/** Deportes (F17): lo que hay en directo ahora y lo de los próximos días, sacado de la guía,
 * con el mismo partido en todos sus canales juntos. Filtros por deporte y por «Mis
 * equipos». */
export function SportsScreen() {
  const reminders = useReminders();
  const { data, isPending } = useSportsEvents(7);
  const [sport, setSport] = useState<string | null>(null);
  const [onlyMine, setOnlyMine] = useState(false);
  const [picking, setPicking] = useState(false);

  const events = data ?? [];
  const sports = useMemo(() => {
    const counts = new Map<string, number>();
    for (const e of events) if (e.sport) counts.set(e.sport, (counts.get(e.sport) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([s]) => s);
  }, [events]);

  const visible = events.filter((e) => (!sport || e.sport === sport) && (!onlyMine || e.followed));
  const now = Date.now() / 1000;
  const live = visible.filter((e) => isOnNow(e, now));
  const byDay = new Map<string, SportEventResponse[]>();
  for (const e of visible) {
    if (isOnNow(e, now)) continue;
    const label = dayLabel(e.start_ts, now);
    byDay.set(label, [...(byDay.get(label) ?? []), e]);
  }

  const chip = (active: boolean) =>
    `shrink-0 rounded-full px-4 py-1.5 text-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/70 ${
      active ? "bg-white text-black" : "bg-white/[0.08] text-neutral-200 hover:bg-white/15"
    }`;

  return (
    <div className="flex flex-col gap-6 px-4 pb-16 pt-6 sm:px-10">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-bold sm:text-3xl ios:text-[34px] ios:tracking-tight">Deportes</h1>
        <button
          type="button"
          onClick={() => setPicking(true)}
          className="ml-auto rounded-full bg-red-600 px-4 py-2 text-sm font-semibold text-white outline-none transition-colors hover:bg-red-500 focus-visible:ring-2 focus-visible:ring-white/70"
        >
          ★ Mis equipos
        </button>
      </header>

      <div className="-mx-4 flex gap-2 overflow-x-auto px-4 pb-1 sm:mx-0 sm:flex-wrap sm:px-0">
        <button type="button" onClick={() => setSport(null)} className={chip(sport === null)}>
          Todo
        </button>
        {sports.map((s) => (
          <button key={s} type="button" onClick={() => setSport(sport === s ? null : s)} className={chip(sport === s)}>
            {sportEmoji(s)} {SPORT_LABELS[s] ?? s}
          </button>
        ))}
        <button type="button" onClick={() => setOnlyMine((v) => !v)} className={chip(onlyMine)} aria-pressed={onlyMine}>
          ★ Solo mis equipos
        </button>
      </div>

      {isPending && <SportsSkeleton />}
      {!isPending && visible.length === 0 && (
        <p className="max-w-lg text-neutral-400">
          {onlyMine ? "Tus equipos no juegan en los próximos días (o la guía aún no lo trae)." : "La guía no trae partidos para estos días. Prueba a sincronizar la guía en Ajustes."}
        </p>
      )}

      {live.length > 0 && <EventSection title="En directo ahora" events={live} reminders={reminders} />}
      {[...byDay.entries()].map(([day, list]) => (
        <EventSection key={day} title={day.charAt(0).toUpperCase() + day.slice(1)} events={list} reminders={reminders} />
      ))}

      {picking && <FollowTeamsSheet onClose={() => setPicking(false)} />}
    </div>
  );
}

function EventSection({ title, events, reminders }: { title: string; events: SportEventResponse[]; reminders: ReturnType<typeof useReminders> }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold text-neutral-200 ios:text-[20px] ios:font-bold ios:text-white">{title}</h2>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
        {events.map((event) => (
          <SportEventCard key={event.key} event={event} reminders={reminders} />
        ))}
      </div>
    </section>
  );
}

function SportsSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="h-36 animate-pulse rounded-2xl bg-white/[0.04]" />
      ))}
    </div>
  );
}
