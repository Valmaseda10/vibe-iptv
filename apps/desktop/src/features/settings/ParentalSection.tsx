import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPut } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { CategoryAdultSetting } from "../../lib/api-types";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { Toggle } from "../../components/Toggle";
import { SkeletonList } from "../../components/Skeleton";
import { BUTTON, INPUT } from "./styles";

const KIND_LABELS: Record<string, string> = { live: "Directo", movie: "Películas", series: "Series" };

/** Ajustes → Control parental (F12): qué categorías son de adultos. Se detectan solas por
 * el nombre al sincronizar; aquí se corrige lo que se escape o lo que se marque de más, y
 * la siguiente sincronización lo respeta. Solo para perfiles de adulto: el núcleo responde
 * 403 a los demás. Los perfiles infantiles y sus PIN se gestionan en el selector de perfiles. */
export function ParentalSection() {
  const queryClient = useQueryClient();
  const { profileId, setProfileId } = useActiveProfile();
  const [filter, setFilter] = useState("");
  const queryKey = ["adult-categories", profileId];
  const categoriesQuery = useQuery({
    queryKey,
    queryFn: () => apiGet<CategoryAdultSetting[]>(`/api/categories/adult?profile_id=${profileId}`),
    enabled: profileId !== null,
    retry: false,
  });
  const mutation = useMutation({
    mutationFn: ({ id, isAdult }: { id: number; isAdult: boolean | null }) => apiPut(`/api/categories/${id}/adult`, { profile_id: profileId, is_adult: isAdult }),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey });
      // Lo que se ve en Directo, Películas y Series depende de esto.
      queryClient.invalidateQueries();
    },
  });

  const profilesCard = (
    <SettingsCard title="Perfiles infantiles">
      <SettingsRow title="Perfiles y PIN" description="Cada perfil puede ser infantil (con edad máxima) y tener su PIN de entrada. Se cambia en el selector de perfiles.">
        <button type="button" className={BUTTON} onClick={() => setProfileId(null)}>
          Abrir el selector
        </button>
      </SettingsRow>
    </SettingsCard>
  );

  if (categoriesQuery.isPending) return <SkeletonList count={3} />;
  if (!categoriesQuery.data) {
    return (
      <div className="flex flex-col gap-6">
        {profilesCard}
        <p className="text-sm text-neutral-400">Las categorías para adultos solo se pueden cambiar desde un perfil de adulto.</p>
      </div>
    );
  }

  const needle = filter.trim().toLowerCase();
  const visible = categoriesQuery.data.filter((c) => !needle || c.name.toLowerCase().includes(needle));
  const markedCount = categoriesQuery.data.filter((c) => c.is_adult).length;

  return (
    <div className="flex flex-col gap-6">
      {profilesCard}
      <SettingsCard title={`Categorías para adultos · ${markedCount}`} footer="Los perfiles que no son de adulto no ven estas categorías. «Automático» es lo que detectó la app por el nombre.">
        <div className="p-3">
          <input type="search" placeholder="Buscar categoría" aria-label="Buscar categoría" className={INPUT} value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
        <ul className="max-h-[28rem] divide-y divide-white/[0.06] overflow-y-auto">
          {visible.map((category) => (
            <li key={category.id} className="flex items-center gap-3 px-4 py-2.5">
              <span className="w-20 shrink-0 text-[12px] text-neutral-500">{KIND_LABELS[category.kind] ?? category.kind}</span>
              <span className="min-w-0 flex-1 truncate text-sm text-neutral-100">{category.name}</span>
              {category.manual ? (
                <button
                  type="button"
                  className="shrink-0 text-[12px] text-neutral-400 underline decoration-dotted outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white/70"
                  onClick={() => mutation.mutate({ id: category.id, isAdult: null })}
                >
                  Volver a automático
                </button>
              ) : (
                <span className="shrink-0 text-[12px] text-neutral-600">automático</span>
              )}
              <Toggle checked={category.is_adult} onChange={(isAdult) => mutation.mutate({ id: category.id, isAdult })} label={`${category.name}: para adultos`} />
            </li>
          ))}
          {visible.length === 0 && <li className="px-4 py-3 text-sm text-neutral-500">Ninguna categoría coincide.</li>}
        </ul>
      </SettingsCard>
    </div>
  );
}
