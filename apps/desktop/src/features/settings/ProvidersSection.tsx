import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { errorMessage } from "../../lib/api-client";
import type { EpgSyncSummary, ProviderSummary, SyncFinished, SyncProgress } from "../../lib/api-types";
import { SkeletonList } from "../../components/Skeleton";
import { PlusIcon, ServerIcon } from "../../components/icons";
import { ProviderCard, type ProviderSyncState } from "./ProviderCard";
import { AddProviderSheet } from "./AddProviderSheet";
import { BUTTON_PRIMARY } from "./styles";

/** Ajustes → Proveedores: una tarjeta por lista con su estado, y «Añadir proveedor» en una
 * hoja aparte que solo sale al pedirla. */
export function ProvidersSection() {
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const [progressByProvider, setProgressByProvider] = useState<Record<number, SyncProgress>>({});
  const [epgSummary, setEpgSummary] = useState<EpgSyncSummary | null>(null);
  // Estado de la última sincronización de cada proveedor, para que «Sincronizar» diga algo
  // al acabar y al fallar (F15).
  const [syncState, setSyncState] = useState<Record<number, ProviderSyncState>>({});

  const providersQuery = useQuery({ queryKey: ["providers"], queryFn: () => invoke<ProviderSummary[]>("list_providers") });

  useEffect(() => {
    const unlisten = listen<SyncProgress>("sync://progress", ({ payload }) => {
      if (payload.stage === "tmdb") return;
      setProgressByProvider((prev) => ({ ...prev, [payload.provider_id]: payload }));
      if (payload.total > 0 && payload.current >= payload.total) queryClient.invalidateQueries({ queryKey: ["providers"] });
    });
    return () => void unlisten.then((fn) => fn());
  }, [queryClient]);

  useEffect(() => {
    const unlisten = listen<SyncFinished>("sync://finished", ({ payload }) => {
      setSyncState((prev) => ({
        ...prev,
        [payload.provider_id]: payload.error
          ? { running: false, failed: true, message: `La sincronización falló: ${payload.error}` }
          : { running: false, failed: false, message: `Sincronizado: ${payload.channels} canales, ${payload.movies} películas, ${payload.series} series.` },
      }));
      queryClient.invalidateQueries({ queryKey: ["providers"] });
    });
    return () => void unlisten.then((fn) => fn());
  }, [queryClient]);

  useEffect(() => {
    const unlisten = listen<EpgSyncSummary>("epg://summary", ({ payload }) => setEpgSummary(payload));
    return () => void unlisten.then((fn) => fn());
  }, []);

  const syncMutation = useMutation({
    mutationFn: (providerId: number) => invoke<void>("sync_provider", { providerId }),
    onMutate: (providerId) => setSyncState((prev) => ({ ...prev, [providerId]: { running: true, failed: false, message: "Sincronizando el catálogo…" } })),
    onError: (err, providerId) => {
      const message = errorMessage(err);
      // Ya había una en marcha (p. ej. la que se lanza al añadir): sigue «en curso».
      const busy = message.includes("Ya se está sincronizando");
      setSyncState((prev) => ({ ...prev, [providerId]: { running: busy, failed: !busy, message } }));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (providerId: number) => invoke<void>("delete_provider", { providerId }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["providers"] }),
  });

  const providers = providersQuery.data ?? [];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm text-neutral-400">
          {providers.length === 1 ? "1 lista" : `${providers.length} listas`} · los datos de acceso se guardan cifrados en este aparato
        </p>
        {providers.length > 0 && (
          <button type="button" className={BUTTON_PRIMARY} onClick={() => setAdding(true)}>
            <PlusIcon className="h-4 w-4" />
            Añadir proveedor
          </button>
        )}
      </div>

      {providersQuery.isPending && <SkeletonList count={2} />}

      {providersQuery.isSuccess && providers.length === 0 && (
        <div className="flex flex-col items-center gap-4 rounded-2xl bg-white/[0.04] px-6 py-10 text-center ring-1 ring-white/[0.06] ios:bg-ios-surface-1 ios:ring-0">
          <span className="flex h-14 w-14 items-center justify-center rounded-2xl bg-red-600/20 text-red-400">
            <ServerIcon className="h-7 w-7" />
          </span>
          <div className="flex flex-col gap-1">
            <p className="text-lg font-semibold text-white">Añade tu primera lista</p>
            <p className="max-w-sm text-sm text-neutral-400">Con los datos de tu proveedor IPTV (Xtream Codes o una lista M3U) cargamos canales, películas y series.</p>
          </div>
          <button type="button" className={BUTTON_PRIMARY} onClick={() => setAdding(true)}>
            <PlusIcon className="h-4 w-4" />
            Añadir proveedor
          </button>
        </div>
      )}

      <ul className="flex flex-col gap-3">
        {providers.map((provider) => (
          <li key={provider.id}>
            <ProviderCard
              provider={provider}
              progress={progressByProvider[provider.id]}
              sync={syncState[provider.id]}
              onSync={() => syncMutation.mutate(provider.id)}
              onDelete={() => deleteMutation.mutate(provider.id)}
              deleting={deleteMutation.isPending && deleteMutation.variables === provider.id}
              deleteError={deleteMutation.isError && deleteMutation.variables === provider.id ? errorMessage(deleteMutation.error) : null}
            />
          </li>
        ))}
      </ul>

      {epgSummary && epgSummary.channels_without_epg > 0 && (
        <p className="text-sm text-amber-400">
          {epgSummary.channels_without_epg} canal{epgSummary.channels_without_epg === 1 ? "" : "es"} sin guía de programación tras la última importación: el proveedor no dio
          un identificador que coincida con la guía.
        </p>
      )}

      {adding && (
        <AddProviderSheet
          onClose={() => setAdding(false)}
          onAdded={(provider) => {
            setAdding(false);
            setSyncState((prev) => ({ ...prev, [provider.id]: { running: true, failed: false, message: "Sincronizando el catálogo…" } }));
            queryClient.invalidateQueries({ queryKey: ["providers"] });
          }}
        />
      )}
    </div>
  );
}
