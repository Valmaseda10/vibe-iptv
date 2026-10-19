import { useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { EpgSearchResult } from "../../lib/api-types";
import type { RemindersApi } from "../../lib/reminders";

/** Resultados de "Buscar en la guía" (F13): lo que se emite ahora o más tarde con ese
 * título. Un mismo programa en varios feeds del mismo canal sale una vez. */
export function GuideSearchResults({ query, reminders }: { query: string; reminders: RemindersApi }) {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const resultsQuery = useQuery({
    queryKey: ["guide-search", profileId, query],
    queryFn: () => apiGet<EpgSearchResult[]>(`/api/epg/search?profile_id=${profileId}&q=${encodeURIComponent(query)}`),
    enabled: profileId !== null,
    staleTime: 30_000,
  });

  const seen = new Set<string>();
  const results = (resultsQuery.data ?? []).filter((r) => {
    const key = `${r.channel.display_name}|${r.start_ts}|${r.title}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  const nowSeconds = Date.now() / 1000;

  if (resultsQuery.isPending) return <p className="px-6 text-neutral-500 sm:px-10">Buscando…</p>;
  if (results.length === 0) return <p className="px-6 text-neutral-500 sm:px-10">Nada en la guía con «{query}».</p>;

  return (
    <ul className="flex-1 overflow-y-auto px-6 sm:px-10">
      {results.map((r) => {
        const airing = r.start_ts <= nowSeconds && r.stop_ts > nowSeconds;
        const reminded = reminders.has(r.channel.id, r.start_ts);
        return (
          <li key={`${r.channel.id}-${r.start_ts}-${r.title}`} className="flex items-center gap-4 border-b border-neutral-900 py-3">
            <div className="w-36 shrink-0 text-sm tabular-nums text-neutral-400">
              {airing ? <span className="font-semibold text-red-500">Ahora</span> : formatDay(r.start_ts)}
              <div className="text-xs text-neutral-500">
                {formatHour(r.start_ts)}–{formatHour(r.stop_ts)}
              </div>
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate font-medium">{r.title}</p>
              <p className="truncate text-sm text-neutral-400">
                {r.channel.number != null ? `${r.channel.number} · ` : ""}
                {r.channel.display_name}
                {r.subtitle ? ` · ${r.subtitle}` : ""}
              </p>
            </div>
            {airing ? (
              <button
                type="button"
                onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(r.channel.id) }, search: {} })}
                className="rounded bg-red-600 px-3 py-1.5 text-sm font-medium outline-none hover:bg-red-500 focus-visible:ring-2 focus-visible:ring-white/70"
              >
                Ver ahora
              </button>
            ) : (
              <button
                type="button"
                onClick={() => reminders.toggle(r.channel, r)}
                className={`rounded px-3 py-1.5 text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-white/70 ${
                  reminded ? "bg-amber-500/20 text-amber-300 hover:bg-amber-500/30" : "bg-neutral-800 hover:bg-neutral-700"
                }`}
              >
                {reminded ? "Recordatorio puesto" : "Recordármelo"}
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function formatHour(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

function formatDay(epochSeconds: number): string {
  const date = new Date(epochSeconds * 1000);
  const today = new Date();
  const tomorrow = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1);
  if (date.toDateString() === today.toDateString()) return "Hoy";
  if (date.toDateString() === tomorrow.toDateString()) return "Mañana";
  return date.toLocaleDateString(undefined, { weekday: "short", day: "numeric" });
}
