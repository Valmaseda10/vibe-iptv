import { useMemo, useState } from "react";
import { GlassSheet } from "../../components/GlassSheet";
import { TeamCrest } from "../../components/TeamCrest";
import { SPORT_LABELS, sportEmoji, useFollowables } from "../../lib/sports";
import type { SportFollow } from "../../lib/api-types";

/** «Mis equipos» (F17): equipos, jugadores y competiciones que sigue el perfil. Lo que se
 * sigue avisa solo cuando juega y, si es importante, sale en la portada de Inicio. Viaja
 * con la cuenta, en cada perfil por separado. */
export function FollowTeamsSheet({ onClose }: { onClose: () => void }) {
  const { list, isPending, toggle } = useFollowables();
  const [filter, setFilter] = useState("");

  const groups = useMemo(() => {
    const query = filter.trim().toLowerCase().normalize("NFD").replace(/\p{Diacritic}/gu, "");
    const visible = list.filter((f) => !query || f.label.toLowerCase().normalize("NFD").replace(/\p{Diacritic}/gu, "").includes(query));
    return [
      { title: "Siguiendo", items: visible.filter((f) => f.followed) },
      { title: "Competiciones", items: visible.filter((f) => !f.followed && f.kind === "competition") },
      { title: "Equipos y deportistas", items: visible.filter((f) => !f.followed && f.kind === "team") },
    ].filter((g) => g.items.length > 0);
  }, [list, filter]);

  return (
    <GlassSheet title="Mis equipos" onClose={onClose}>
      <div className="flex max-h-[70vh] flex-col gap-4 overflow-y-auto px-1 pb-4">
        <p className="text-sm text-neutral-400">Te avisaremos cuando jueguen y sus partidos importantes saldrán en Inicio.</p>
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Buscar equipo, jugador o competición…"
          className="w-full rounded-lg border border-neutral-700 bg-neutral-900 px-3 py-2 text-sm outline-none focus-visible:border-red-500 ios:rounded-full ios:border-white/10 ios:bg-white/[0.06]"
          autoFocus
        />
        {isPending && <p className="text-sm text-neutral-500">Cargando…</p>}
        {groups.map((group) => (
          <section key={group.title} className="flex flex-col gap-1">
            <h3 className="px-1 text-xs font-semibold uppercase tracking-wide text-neutral-500">{group.title}</h3>
            {group.items.slice(0, 60).map((item) => (
              <FollowRow key={`${item.kind}:${item.key}`} item={item} onToggle={() => toggle(item)} />
            ))}
          </section>
        ))}
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
        <TeamCrest teamKey={item.key} name={item.label} size="sm" />
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
