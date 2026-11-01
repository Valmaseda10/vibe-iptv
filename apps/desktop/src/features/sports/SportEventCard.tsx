import { useNavigate } from "@tanstack/react-router";
import type { SportEventResponse } from "../../lib/api-types";
import type { RemindersApi } from "../../lib/reminders";
import { isOnNow, sportEmoji, timeLabel } from "../../lib/sports";
import { TeamCrest } from "../../components/TeamCrest";
import { BellIcon } from "../../components/icons";

/** Un partido o carrera en la lista de Deportes: hora (o EN DIRECTO), competición, rivales
 * con sus escudos, los canales que lo dan y la campana para que avise. */
export function SportEventCard({ event, reminders }: { event: SportEventResponse; reminders: RemindersApi }) {
  const navigate = useNavigate();
  const live = isOnNow(event);
  const first = event.channels[0];
  const reminded = first ? reminders.has(first.id, event.start_ts) : false;
  const play = (channelId: number) => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(channelId) } });
  const [home, away] = event.participants;

  return (
    <article
      className={`flex flex-col gap-3 rounded-2xl p-4 ring-1 transition-colors ${
        event.followed ? "bg-red-950/30 ring-red-600/40" : "bg-white/[0.04] ring-white/10"
      } ${live ? "shadow-[0_0_0_1px_rgba(220,38,38,0.35)]" : ""}`}
    >
      <header className="flex items-center gap-2 text-xs">
        {live ? (
          <span className="flex items-center gap-1.5 rounded bg-red-600 px-1.5 py-0.5 font-bold uppercase tracking-wide text-white">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-white" />
            En directo
          </span>
        ) : (
          <span className="font-semibold tabular-nums text-neutral-200">{timeLabel(event.start_ts)}</span>
        )}
        <span className="truncate text-neutral-400">
          {sportEmoji(event.sport)} {event.competition ?? ""}
        </span>
        {event.followed && <span className="ml-auto shrink-0 text-[11px] font-semibold text-red-400">★ Tu equipo</span>}
      </header>

      {home && away ? (
        <div className="flex items-center gap-3">
          <TeamCrest teamKey={home.key} name={home.name} />
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-semibold text-white">{home.name}</span>
            <span className="truncate font-semibold text-white">{away.name}</span>
          </div>
          <TeamCrest teamKey={away.key} name={away.name} />
        </div>
      ) : (
        <p className="font-semibold text-white">{event.name}</p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {event.channels.slice(0, 4).map((channel) => (
          <button
            key={channel.id}
            type="button"
            onClick={() => play(channel.id)}
            className="flex items-center gap-1.5 rounded-full bg-white/10 py-1 pl-1 pr-3 text-xs text-neutral-100 outline-none transition-colors hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white/70"
          >
            <span className="flex h-5 w-5 items-center justify-center overflow-hidden rounded-full bg-neutral-900">
              {channel.logo_url ? <img src={channel.logo_url} alt="" className="h-full w-full object-contain" /> : <span className="text-[9px]">TV</span>}
            </span>
            {live ? "▶ " : ""}
            {channel.name}
          </button>
        ))}
        {event.channels.length > 4 && <span className="text-xs text-neutral-500">+{event.channels.length - 4}</span>}
        {!live && first && (
          <button
            type="button"
            onClick={() => reminders.toggle({ id: first.id }, { title: event.name, start_ts: event.start_ts, stop_ts: event.stop_ts })}
            aria-pressed={reminded}
            aria-label={reminded ? "Quitar el aviso" : "Avísame cuando empiece"}
            className={`ml-auto flex items-center gap-1.5 rounded-full px-3 py-1 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/70 ${
              reminded ? "bg-amber-500/20 text-amber-300" : "text-neutral-400 hover:bg-white/10 hover:text-white"
            }`}
          >
            <BellIcon className="h-3.5 w-3.5" />
            {reminded ? "Te avisaré" : "Avísame"}
          </button>
        )}
      </div>
    </article>
  );
}
