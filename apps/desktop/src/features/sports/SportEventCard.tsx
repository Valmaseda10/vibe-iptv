import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import type { SportEventResponse } from "../../lib/api-types";
import type { RemindersApi } from "../../lib/reminders";
import { isLive, isOnNow, sportEmoji, timeLabel } from "../../lib/sports";
import { TeamCrest } from "../../components/TeamCrest";
import { BellIcon } from "../../components/icons";
import { ChannelLogo } from "../../components/ChannelLogo";
import { SportEventSheet } from "./SportEventSheet";

/** Un partido o carrera en Deportes (F17). Tocarlo, si ya se emite, abre el mejor canal (el
 * núcleo los da ordenados: tus favoritos y la mejor calidad, primero); si aún no empieza,
 * enseña sus canales y «Avísame». «Otros canales», aparte. */
export function SportEventCard({ event, reminders }: { event: SportEventResponse; reminders: RemindersApi }) {
  const navigate = useNavigate();
  const [sheetOpen, setSheetOpen] = useState(false);
  const live = isLive(event);
  const onNow = isOnNow(event);
  const first = event.channels[0];
  const reminded = first ? reminders.has(first.id, event.start_ts) : false;
  const [home, away] = event.participants;
  const hasScore = home?.score != null && away?.score != null;

  function open() {
    if (onNow && first) navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(first.id) } });
    else setSheetOpen(true);
  }

  const badge = live ? (
    <span className="flex items-center gap-1.5 rounded bg-red-600 px-1.5 py-0.5 font-bold uppercase tracking-wide text-white">
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-white" />
      En directo{event.clock ? ` · ${event.clock}` : ""}
    </span>
  ) : event.status === "replay" ? (
    <span className="rounded bg-neutral-700 px-1.5 py-0.5 font-bold uppercase tracking-wide text-neutral-200">Diferido</span>
  ) : onNow ? (
    <span className="rounded bg-white/15 px-1.5 py-0.5 font-bold uppercase tracking-wide text-white">Ahora</span>
  ) : (
    <span className="font-semibold tabular-nums text-neutral-200">{timeLabel(event.start_ts)}</span>
  );

  return (
    <>
      <article
        role="button"
        tabIndex={0}
        onClick={open}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            open();
          }
        }}
        aria-label={`${event.name}${onNow ? ", ver ahora" : ", ver canales"}`}
        className={`flex cursor-pointer flex-col gap-3 rounded-2xl p-4 outline-none ring-1 transition-[background-color,transform] hover:bg-white/[0.07] focus-visible:ring-2 focus-visible:ring-white/70 active:scale-[0.99] ${
          event.followed ? "bg-red-950/30 ring-red-600/40" : "bg-white/[0.04] ring-white/10"
        } ${event.status === "replay" ? "opacity-75" : ""}`}
      >
        <header className="flex items-center gap-2 text-xs">
          {badge}
          <span className="flex min-w-0 items-center gap-1.5 truncate text-neutral-400">
            {event.league_logo ? <img src={event.league_logo} alt="" className="h-4 w-4 object-contain" /> : <span>{sportEmoji(event.sport)}</span>}
            {event.competition ?? ""}
          </span>
          {event.followed && <span className="ml-auto shrink-0 text-[11px] font-semibold text-red-400">★ Tu equipo</span>}
        </header>

        {home && away ? (
          <div className="flex items-center gap-3">
            <TeamCrest teamKey={home.key} name={home.name} logo={home.logo} />
            <div className="flex min-w-0 flex-1 flex-col">
              <span className="flex items-center justify-between gap-2 font-semibold text-white">
                <span className="truncate">{home.name}</span>
                {hasScore && <span className="tabular-nums">{home.score}</span>}
              </span>
              <span className="flex items-center justify-between gap-2 font-semibold text-white">
                <span className="truncate">{away.name}</span>
                {hasScore && <span className="tabular-nums">{away.score}</span>}
              </span>
            </div>
            <TeamCrest teamKey={away.key} name={away.name} logo={away.logo} />
          </div>
        ) : (
          <p className="font-semibold text-white">{event.name}</p>
        )}

        <div className="flex items-center gap-2 text-xs text-neutral-400">
          {first && (
            <span className="flex min-w-0 items-center gap-1.5">
              <ChannelLogo url={first.logo_url} name={first.name} />
              <span className="truncate">
                {onNow ? "▶ " : ""}
                {first.name}
              </span>
            </span>
          )}
          {event.channels.length > 1 && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setSheetOpen(true);
              }}
              className="shrink-0 rounded-full bg-white/10 px-2.5 py-1 text-neutral-200 outline-none hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white/70"
            >
              +{event.channels.length - 1} canales
            </button>
          )}
          {!onNow && first && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                reminders.toggle({ id: first.id }, { title: event.name, start_ts: event.start_ts, stop_ts: event.stop_ts });
              }}
              aria-pressed={reminded}
              aria-label={reminded ? "Quitar el aviso" : "Avísame cuando empiece"}
              className={`ml-auto flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/70 ${
                reminded ? "bg-amber-500/20 text-amber-300" : "hover:bg-white/10 hover:text-white"
              }`}
            >
              <BellIcon className="h-3.5 w-3.5" />
              {reminded ? "Te avisaré" : "Avísame"}
            </button>
          )}
        </div>
      </article>
      {sheetOpen && <SportEventSheet event={event} reminders={reminders} onClose={() => setSheetOpen(false)} />}
    </>
  );
}
