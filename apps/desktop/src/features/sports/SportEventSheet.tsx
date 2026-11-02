import { useNavigate } from "@tanstack/react-router";
import { GlassSheet } from "../../components/GlassSheet";
import { TeamCrest } from "../../components/TeamCrest";
import { BellIcon } from "../../components/icons";
import { ChannelLogo } from "../../components/ChannelLogo";
import type { SportEventResponse } from "../../lib/api-types";
import type { RemindersApi } from "../../lib/reminders";
import { dayLabel, isOnNow, sportEmoji, timeLabel, whenLabel } from "../../lib/sports";

/** Un partido con todos sus canales (F17): al tocar uno que aún no ha empezado, o «Otros
 * canales» de uno en juego. Cada canal se puede abrir; si no ha empezado, «Avísame». */
export function SportEventSheet({ event, reminders, onClose }: { event: SportEventResponse; reminders: RemindersApi; onClose: () => void }) {
  const navigate = useNavigate();
  const [home, away] = event.participants;
  const first = event.channels[0];
  const started = isOnNow(event);
  const reminded = first ? reminders.has(first.id, event.start_ts) : false;
  const play = (id: number) => {
    onClose();
    navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(id) } });
  };

  return (
    <GlassSheet title={event.name} showTitle={false} onClose={onClose}>
      <div className="flex flex-col gap-5 px-1 pb-4">
        <p className="text-center text-xs font-semibold uppercase tracking-wide text-neutral-400">
          {sportEmoji(event.sport)} {event.competition ?? "Deportes"} · {started ? whenLabel(event) : `${dayLabel(event.start_ts)} · ${timeLabel(event.start_ts)}`}
        </p>
        {home && away ? (
          <div className="flex items-center justify-center gap-4">
            <div className="flex w-28 flex-col items-center gap-2 text-center">
              <TeamCrest teamKey={home.key} name={home.name} logo={home.logo} size="lg" />
              <span className="text-sm font-semibold text-white">{home.name}</span>
            </div>
            <span className="text-2xl font-bold tabular-nums text-white">{home.score != null && away.score != null ? `${home.score} - ${away.score}` : "vs"}</span>
            <div className="flex w-28 flex-col items-center gap-2 text-center">
              <TeamCrest teamKey={away.key} name={away.name} logo={away.logo} size="lg" />
              <span className="text-sm font-semibold text-white">{away.name}</span>
            </div>
          </div>
        ) : (
          <p className="text-center text-xl font-bold text-white">{event.name}</p>
        )}

        <div className="flex flex-col gap-1">
          <p className="px-1 text-xs font-semibold uppercase tracking-wide text-neutral-500">Dónde verlo</p>
          {event.channels.map((channel, index) => (
            <button
              key={channel.id}
              type="button"
              onClick={() => play(channel.id)}
              autoFocus={index === 0}
              className="flex items-center gap-3 rounded-xl px-2 py-2.5 text-left outline-none transition-colors hover:bg-white/[0.06] focus-visible:bg-white/10"
            >
              <ChannelLogo url={channel.logo_url} className="h-9 w-9" />
              <span className="flex-1 text-sm font-medium text-white">{channel.name}</span>
              {index === 0 && <span className="text-xs text-neutral-500">Recomendado</span>}
              <span className="text-neutral-400">▶</span>
            </button>
          ))}
        </div>

        {!started && first && (
          <button
            type="button"
            onClick={() => reminders.toggle({ id: first.id }, { title: event.name, start_ts: event.start_ts, stop_ts: event.stop_ts })}
            aria-pressed={reminded}
            className={`flex items-center justify-center gap-2 rounded-full py-3 text-sm font-semibold outline-none focus-visible:ring-2 focus-visible:ring-white/70 ${
              reminded ? "bg-amber-500/20 text-amber-300" : "bg-white text-black"
            }`}
          >
            <BellIcon className="h-4 w-4" />
            {reminded ? "Te avisaré cuando empiece" : "Avísame cuando empiece"}
          </button>
        )}
      </div>
    </GlassSheet>
  );
}
