import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import type { SportEventResponse } from "../../lib/api-types";
import { useReminders } from "../../lib/reminders";
import { IS_IOS } from "../../lib/platform";
import { isOnNow, sportEmoji, whenLabel } from "../../lib/sports";
import { TeamCrest } from "../../components/TeamCrest";
import { BellIcon } from "../../components/icons";

/** El partido importante en la portada de Inicio (F17), como Movistar Plus: desde unas
 * horas antes hasta que acaba, en el sitio de la película o serie destacada. */
export function MatchHero({ event }: { event: SportEventResponse }) {
  const navigate = useNavigate();
  const reminders = useReminders();
  // La cuenta («Empieza en 12 min») se pone al día sola.
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now() / 1000), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const live = isOnNow(event, now);
  const first = event.channels[0];
  const soon = event.start_ts - now <= 15 * 60;
  const reminded = first ? reminders.has(first.id, event.start_ts) : false;
  const [home, away] = event.participants;
  const watch = () => first && navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(first.id) } });

  const primary = IS_IOS
    ? "flex items-center gap-2 rounded-full bg-white px-6 py-3 text-[17px] font-semibold text-black outline-none active:bg-neutral-300"
    : "flex items-center gap-2 rounded-lg bg-white px-6 py-2.5 font-semibold text-black outline-none transition hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-red-600";
  const secondary = IS_IOS
    ? "glass-clear glass-pressable flex items-center gap-2 rounded-full px-5 py-3 text-[17px] font-semibold text-white outline-none"
    : "flex items-center gap-2 rounded-lg bg-neutral-700/80 px-5 py-2.5 font-semibold text-white outline-none transition hover:bg-neutral-600/80 focus-visible:ring-2 focus-visible:ring-red-600";

  return (
    <div className="relative h-[56vh] min-h-[340px] w-full overflow-hidden sm:h-[62vh] sm:min-h-[380px]">
      {/* Estadio de noche: césped, focos y las líneas del campo. */}
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_50%_-10%,rgba(255,255,255,0.22),transparent_55%),linear-gradient(180deg,#0b3d2a_0%,#0a2e20_45%,#06140e_100%)]" />
      <div className="absolute inset-0 opacity-[0.12] [background-image:repeating-linear-gradient(90deg,transparent_0_80px,rgba(255,255,255,0.5)_80px_160px)]" />
      <svg aria-hidden viewBox="0 0 400 200" className="absolute inset-x-0 bottom-0 h-3/4 w-full opacity-20" preserveAspectRatio="none">
        <g fill="none" stroke="white" strokeWidth="1.2">
          <rect x="20" y="20" width="360" height="170" />
          <line x1="200" y1="20" x2="200" y2="190" />
          <circle cx="200" cy="105" r="32" />
          <rect x="20" y="70" width="44" height="70" />
          <rect x="336" y="70" width="44" height="70" />
        </g>
      </svg>
      <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/30 to-transparent ios:from-black ios:via-black/30" />

      <div className="absolute inset-x-0 bottom-0 flex flex-col gap-4 px-4 pb-8 sm:px-10 sm:pb-10">
        <p className="flex items-center gap-2 text-sm font-semibold text-neutral-100 drop-shadow">
          {live ? (
            <span className="flex items-center gap-1.5 rounded bg-red-600 px-1.5 py-0.5 text-[11px] font-bold uppercase leading-none tracking-wide text-white">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-white" />
              En directo
            </span>
          ) : (
            <span className="rounded bg-white/15 px-1.5 py-0.5 text-[11px] font-bold uppercase leading-none tracking-wide text-white">{whenLabel(event, now)}</span>
          )}
          <span>
            {sportEmoji(event.sport)} {event.competition ?? "Deportes"}
            {event.followed ? " · Tu equipo" : ""}
          </span>
        </p>

        {home && away ? (
          <div className="flex items-center gap-3 sm:gap-6">
            <TeamCrest teamKey={home.key} name={home.name} size="lg" />
            <h2 className="flex min-w-0 flex-col text-2xl font-bold leading-tight drop-shadow-lg sm:flex-row sm:items-center sm:gap-4 sm:text-5xl">
              <span className="truncate">{home.name}</span>
              <span className="text-base font-semibold text-neutral-300 sm:text-2xl">vs</span>
              <span className="truncate">{away.name}</span>
            </h2>
            <TeamCrest teamKey={away.key} name={away.name} size="lg" />
          </div>
        ) : (
          <h2 className="text-3xl font-bold drop-shadow-lg sm:text-5xl">{event.name}</h2>
        )}

        {event.channels.length > 0 && (
          <p className="text-sm text-neutral-300">
            En {event.channels.slice(0, 3).map((c) => c.name).join(", ")}
            {event.channels.length > 3 ? ` y ${event.channels.length - 3} más` : ""}
          </p>
        )}

        <div className="flex flex-wrap items-center gap-3">
          {(live || soon) && first ? (
            <button type="button" onClick={watch} className={primary} autoFocus>
              ▶ {live ? "Ver en directo" : "Ir al canal"}
            </button>
          ) : (
            first && (
              <button
                type="button"
                onClick={() => reminders.toggle({ id: first.id }, { title: event.name, start_ts: event.start_ts, stop_ts: event.stop_ts })}
                aria-pressed={reminded}
                className={primary}
              >
                <BellIcon className="h-5 w-5" />
                {reminded ? "Te avisaré" : "Avísame"}
              </button>
            )
          )}
          <button type="button" onClick={() => navigate({ to: "/sports" })} className={secondary}>
            Más deportes
          </button>
        </div>
      </div>
    </div>
  );
}
