import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import type { SportEventResponse } from "../../lib/api-types";
import { useReminders } from "../../lib/reminders";
import { IS_IOS } from "../../lib/platform";
import { crestColor, sportEmoji } from "../../lib/sports";
import { TeamCrest } from "../../components/TeamCrest";
import { SportEventSheet } from "./SportEventSheet";

/** El partido en juego en la portada de Inicio (F17), como Movistar Plus: solo cuando se
 * está jugando de verdad y es de los que quiere el perfil (lo decide el núcleo). Fondo con
 * los colores de los dos equipos, sus escudos, el marcador y el minuto. */
export function MatchHero({ event }: { event: SportEventResponse }) {
  const navigate = useNavigate();
  const reminders = useReminders();
  const [sheetOpen, setSheetOpen] = useState(false);
  const first = event.channels[0];
  const [home, away] = event.participants;
  const homeColor = home?.color ?? (home ? crestColor(home.key) : "#0b3d2a");
  const awayColor = away?.color ?? (away ? crestColor(away.key) : "#0b3d2a");
  const hasScore = home?.score != null && away?.score != null;
  const watch = () => first && navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(first.id) } });

  const primary = IS_IOS
    ? "flex items-center gap-2 rounded-full bg-white px-6 py-3 text-[17px] font-semibold text-black outline-none active:bg-neutral-300"
    : "flex items-center gap-2 rounded-lg bg-white px-6 py-2.5 font-semibold text-black outline-none transition hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-red-600";
  const secondary = IS_IOS
    ? "glass-clear glass-pressable flex items-center gap-2 rounded-full px-5 py-3 text-[17px] font-semibold text-white outline-none"
    : "flex items-center gap-2 rounded-lg bg-neutral-700/80 px-5 py-2.5 font-semibold text-white outline-none transition hover:bg-neutral-600/80 focus-visible:ring-2 focus-visible:ring-red-600";

  return (
    <div className="relative h-[56vh] min-h-[360px] w-full overflow-hidden sm:h-[62vh] sm:min-h-[400px]">
      {/* Los colores de los dos equipos, en diagonal, con luz de estadio por arriba. */}
      <div className="absolute inset-0" style={{ background: `linear-gradient(115deg, ${homeColor} 0%, ${homeColor} 42%, ${awayColor} 58%, ${awayColor} 100%)` }} />
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_50%_-20%,rgba(255,255,255,0.35),transparent_60%)]" />
      <div className="absolute inset-0 bg-black/45" />
      <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/30 to-transparent ios:from-black ios:via-black/30" />

      <div className="absolute inset-x-0 bottom-0 flex flex-col items-center gap-5 px-4 pb-8 text-center sm:px-10 sm:pb-10">
        <p className="flex items-center gap-2 text-sm font-semibold text-neutral-100 drop-shadow">
          <span className="flex items-center gap-1.5 rounded bg-red-600 px-1.5 py-0.5 text-[11px] font-bold uppercase leading-none tracking-wide text-white">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-white" />
            En directo{event.clock ? ` · ${event.clock}` : ""}
          </span>
          {event.league_logo ? <img src={event.league_logo} alt="" className="h-5 w-5 object-contain" /> : <span>{sportEmoji(event.sport)}</span>}
          <span>
            {event.competition ?? "Deportes"}
            {event.followed ? " · Tu equipo" : ""}
          </span>
        </p>

        {home && away ? (
          <div className="flex w-full max-w-4xl items-center justify-center gap-4 sm:gap-10">
            <div className="flex min-w-0 flex-1 flex-col items-center gap-3">
              <TeamCrest teamKey={home.key} name={home.name} logo={home.logo} size="xl" />
              <span className="truncate text-lg font-bold drop-shadow sm:text-3xl">{home.name}</span>
            </div>
            <div className="flex shrink-0 flex-col items-center">
              <span className="text-4xl font-black tabular-nums drop-shadow-lg sm:text-7xl">{hasScore ? `${home.score} - ${away.score}` : "vs"}</span>
            </div>
            <div className="flex min-w-0 flex-1 flex-col items-center gap-3">
              <TeamCrest teamKey={away.key} name={away.name} logo={away.logo} size="xl" />
              <span className="truncate text-lg font-bold drop-shadow sm:text-3xl">{away.name}</span>
            </div>
          </div>
        ) : (
          <h2 className="text-3xl font-bold drop-shadow-lg sm:text-5xl">{event.name}</h2>
        )}

        <div className="flex flex-wrap items-center justify-center gap-3">
          {first && (
            <button type="button" onClick={watch} className={primary} autoFocus>
              ▶ Ver en {first.name}
            </button>
          )}
          {event.channels.length > 1 && (
            <button type="button" onClick={() => setSheetOpen(true)} className={secondary}>
              Otros canales
            </button>
          )}
          <button type="button" onClick={() => navigate({ to: "/sports" })} className={secondary}>
            Más deportes
          </button>
        </div>
      </div>
      {sheetOpen && <SportEventSheet event={event} reminders={reminders} onClose={() => setSheetOpen(false)} />}
    </div>
  );
}
