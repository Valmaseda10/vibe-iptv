import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, imagePath, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { BrowseItem, RatedTitle, TasteCandidatesResponse } from "../../lib/api-types";
import { RATING_OPTIONS, useRatings, type RatingValue } from "../../lib/ratings";
import { TastePicker } from "../profiles/TastePicker";

/** Ajustes → «Mis gustos»: lo que el perfil ha valorado (lo que más pesa en sus
 * recomendaciones), para cambiarlo o quitarlo, y «Elegir más títulos». */
export function TasteSection() {
  const { profileId } = useActiveProfile();
  const { data: base } = useApiBase();
  const queryClient = useQueryClient();
  const { setRating } = useRatings();
  const [picking, setPicking] = useState(false);
  const [saving, setSaving] = useState(false);

  const rated = useQuery({
    queryKey: ["ratings", profileId, "titles"],
    queryFn: () => apiGet<RatedTitle[]>(`/api/taste/ratings?profile_id=${profileId}`),
    enabled: profileId !== null,
  });
  const candidates = useQuery({
    queryKey: ["taste-candidates", profileId, "more"],
    queryFn: () => apiGet<TasteCandidatesResponse>(`/api/taste/candidates?profile_id=${profileId}&force=1&seed=${Math.floor(Math.random() * 2 ** 32)}`),
    enabled: picking && profileId !== null,
    staleTime: Infinity,
  });

  async function saveMore(picked: BrowseItem[]) {
    if (profileId === null) return;
    setSaving(true);
    await Promise.all(picked.map((item) => apiPost("/api/ratings", { profile_id: profileId, item_kind: item.kind, item_id: item.id, rating: 1 }))).catch(() => {});
    setSaving(false);
    setPicking(false);
    for (const key of ["ratings", "browse", "home", "taste-candidates"]) queryClient.invalidateQueries({ queryKey: [key] });
  }

  const titles = rated.data ?? [];
  const groups = ([2, 1, -1] as RatingValue[]).map((value) => ({
    value,
    option: RATING_OPTIONS.find((o) => o.value === value),
    titles: titles.filter((t) => t.rating === value),
  }));

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-4 sm:p-6 ios:mb-8 ios:rounded-[22px] ios:border-0 ios:bg-ios-surface-1 ios:p-5">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-xl font-medium ios:text-[22px] ios:font-bold ios:tracking-tight">Mis gustos</h2>
        <button
          type="button"
          onClick={() => setPicking(true)}
          className="rounded-lg bg-white/10 px-4 py-2 text-sm font-medium text-white outline-none hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-full"
        >
          Elegir más títulos
        </button>
      </div>
      <p className="mb-4 text-sm text-neutral-400">Lo que has valorado es lo que más pesa en tus recomendaciones. Cámbialo o quítalo cuando quieras.</p>

      {titles.length === 0 && !rated.isPending && <p className="text-sm text-neutral-500">Todavía no has valorado nada.</p>}

      {groups
        .filter((group) => group.titles.length > 0)
        .map((group) => (
          <div key={group.value} className="mb-4">
            <h3 className="mb-2 text-sm font-semibold text-neutral-300">
              {group.option?.icon} {group.option?.label} · {group.titles.length}
            </h3>
            <ul className="flex flex-col gap-1.5">
              {group.titles.map((title) => {
                const poster = imagePath(base, title.poster_path, "w342");
                return (
                  <li key={`${title.kind}-${title.id}`} className="flex items-center gap-3 rounded-lg bg-black/20 p-1.5 pr-2">
                    {poster ? <img src={poster} alt="" className="h-12 w-8 shrink-0 rounded object-cover" /> : <span className="h-12 w-8 shrink-0 rounded bg-neutral-800" />}
                    <span className="min-w-0 flex-1 truncate text-sm text-neutral-100">
                      {title.clean_title}
                      <span className="ml-2 text-xs text-neutral-500">{title.kind === "series" ? "Serie" : "Película"}</span>
                    </span>
                    <select
                      value={title.rating}
                      onChange={(e) => {
                        const value = e.target.value === "none" ? null : (Number(e.target.value) as RatingValue);
                        setRating(title.kind, title.id, value);
                        queryClient.invalidateQueries({ queryKey: ["ratings", profileId, "titles"] });
                      }}
                      aria-label={`Valoración de ${title.clean_title}`}
                      className="rounded bg-neutral-800 px-2 py-1 text-xs text-neutral-200 outline-none focus:ring-2 focus:ring-red-600 ios:rounded-full ios:bg-white/10"
                    >
                      {RATING_OPTIONS.map((option) => (
                        <option key={option.value} value={option.value}>
                          {option.icon} {option.label}
                        </option>
                      ))}
                      <option value="none">Quitar</option>
                    </select>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}

      {picking && candidates.data && candidates.data.items.length > 0 && (
        <TastePicker items={candidates.data.items} saving={saving} onDone={(picked) => void saveMore(picked)} onSkip={() => setPicking(false)} />
      )}
    </section>
  );
}
