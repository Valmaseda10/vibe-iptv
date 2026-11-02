import { useMemo, useState } from "react";
import { GlassSheet } from "../../components/GlassSheet";
import { TeamCrest } from "../../components/TeamCrest";
import { SkeletonList } from "../../components/Skeleton";
import { Toggle } from "../../components/Toggle";
import { SPORT_LABELS, sportEmoji, useFollowables, useHomePreference } from "../../lib/sports";
import type { SportFollow } from "../../lib/api-types";

/** Orden de los grupos de «Mis equipos». Lo que no esté aquí va al final. */
const GROUP_ORDER = ["LaLiga", "LaLiga Hypermotion", "Primera Federación", "Selecciones", "Competiciones", "Otros equipos", "En tu guía"];

const fold = (text: string) => text.toLowerCase().normalize("NFD").replace(/\p{Diacritic}/gu, "");

/** «Mis equipos» (F17): equipos, deportistas y competiciones que sigue el perfil. Lo que se
 * sigue sale destacado en Deportes y, en directo, en la portada de Inicio; avisar es aparte
 * (el interruptor de arriba, el mismo que en Ajustes → Avisos). Viaja con la cuenta, en cada
 * perfil por separado. */
export function FollowTeamsSheet({ onClose }: { onClose: () => void }) {
  const { list, isPending, toggle } = useFollowables();
  const home = useHomePreference();
  const [filter, setFilter] = useState("");

  const groups = useMemo(() => {
    const query = fold(filter.trim());
    const visible = list.filter((f) => !query || fold(f.label).includes(query));
    const byGroup = new Map<string, SportFollow[]>();
    for (const item of visible.filter((f) => !f.followed)) {
      const group = item.group ?? "Otros equipos";
      byGroup.set(group, [...(byGroup.get(group) ?? []), item]);
    }
    const rank = (group: string) => (GROUP_ORDER.indexOf(group) === -1 ? GROUP_ORDER.length : GROUP_ORDER.indexOf(group));
    const sorted = [...byGroup.entries()]
      .sort((a, b) => rank(a[0]) - rank(b[0]))
      .map(([title, items]) => ({
        title,
        // Las ligas, por orden alfabético; lo de la guía, como viene (lo que más sale, primero).
        items: title === "En tu guía" || title === "Competiciones" ? items : [...items].sort((a, b) => a.label.localeCompare(b.label, "es")),
      }));
    const followed = visible.filter((f) => f.followed);
    return followed.length > 0 ? [{ title: "Siguiendo", items: followed }, ...sorted] : sorted;
  }, [list, filter]);

  return (
    <GlassSheet title="Mis equipos" onClose={onClose}>
      <div className="flex max-h-[75vh] flex-col gap-4 overflow-y-auto px-1 pb-4 [&>*]:shrink-0">
        <p className="px-1 text-sm text-neutral-400">Sus partidos salen destacados aquí y, en directo, en la portada de Inicio.</p>

        <div className="flex flex-col divide-y divide-white/[0.06] overflow-hidden rounded-xl bg-white/[0.05]">
          <div className="flex flex-col gap-2 p-3">
            <p className="text-sm font-medium text-white">Partidos en Inicio (en directo)</p>
            <div role="radiogroup" aria-label="Partidos en Inicio" className="grid grid-cols-3 gap-1 rounded-xl bg-black/30 p-1">
              {[
                { value: "mine", label: "Mis equipos" },
                { value: "mine_and_big", label: "Mis equipos y los grandes" },
                { value: "never", label: "Nunca" },
              ].map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={home.value === option.value}
                  onClick={() => home.set(option.value)}
                  className={`rounded-lg px-2 py-2 text-xs font-medium leading-tight outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/70 ${
                    home.value === option.value ? "bg-white text-black" : "text-neutral-300 hover:bg-white/10"
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <p className="text-xs text-neutral-500">«Los grandes»: Madrid, Barça y Atlético en LaLiga, la Copa y la Champions, y la selección.</p>
          </div>
          <div className="flex items-center gap-3 p-3">
            <div className="flex min-w-0 flex-1 flex-col">
              <span className="text-sm font-medium text-white">Avisarme cuando jueguen</span>
              <span className="text-xs text-neutral-500">Un aviso 5 minutos antes. También en Ajustes → Avisos.</span>
            </div>
            <Toggle checked={home.notify} onChange={home.setNotify} disabled={!home.loaded} label="Avisarme cuando jueguen" />
          </div>
        </div>

        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Buscar equipo, jugador o competición…"
          aria-label="Buscar equipo, jugador o competición"
          className="w-full rounded-lg border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none focus-visible:border-red-500 ios:rounded-full ios:border-white/10 ios:bg-white/[0.06]"
        />
        {isPending && <SkeletonList count={6} />}
        {groups.map((group) => (
          <section key={group.title} className="flex flex-col gap-1">
            <h3 className="px-1 text-xs font-semibold uppercase tracking-wide text-neutral-500">
              {group.title} · {group.items.length}
            </h3>
            {group.items.slice(0, filter ? 200 : 60).map((item) => (
              <FollowRow key={`${item.kind}:${item.key}`} item={item} onToggle={() => toggle(item)} />
            ))}
          </section>
        ))}
        {!isPending && groups.length === 0 && <p className="px-1 text-sm text-neutral-500">Nada coincide con «{filter}».</p>}
      </div>
    </GlassSheet>
  );
}

function FollowRow({ item, onToggle }: { item: SportFollow; onToggle: () => void }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-pressed={item.followed}
      className="flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left outline-none transition-colors hover:bg-white/[0.06] focus-visible:bg-white/10"
    >
      {item.kind === "team" ? (
        <TeamCrest teamKey={item.key} name={item.label} logo={item.logo} size="sm" />
      ) : (
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white/10 text-base">{sportEmoji(item.sport)}</span>
      )}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium text-white">{item.label}</span>
        {item.sport && <span className="text-xs text-neutral-500">{SPORT_LABELS[item.sport] ?? item.sport}</span>}
      </span>
      <span
        className={`flex h-7 shrink-0 items-center rounded-full px-3 text-xs font-semibold transition-colors ${item.followed ? "bg-red-600 text-white" : "bg-white/10 text-neutral-300"}`}
      >
        {item.followed ? "Siguiendo" : "Seguir"}
      </span>
    </button>
  );
}
