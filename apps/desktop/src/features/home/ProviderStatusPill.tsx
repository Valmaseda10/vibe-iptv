import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import type { ProviderAvailability } from "../../lib/api-types";

/** Indicador discreto de si la lista del proveedor responde ahora mismo (F15): verde si sí,
 * rojo si no. Tocándolo dice por qué (p. ej. bloqueo por país sin la VPN) y deja
 * reintentar. El núcleo guarda la comprobación unos minutos para no castigar al panel. */
export function ProviderStatusPill() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const statusQuery = useQuery({
    queryKey: ["provider-status"],
    queryFn: () => apiGet<ProviderAvailability[]>("/api/providers/status"),
    staleTime: 3 * 60_000,
    refetchOnWindowFocus: true,
  });

  const providers = statusQuery.data ?? [];
  if (providers.length === 0) return null;
  const down = providers.filter((p) => !p.available);
  const ok = down.length === 0;

  async function retry() {
    setRetrying(true);
    try {
      const fresh = await apiGet<ProviderAvailability[]>("/api/providers/status?force=true");
      queryClient.setQueryData(["provider-status"], fresh);
      if (fresh.every((p) => p.available)) queryClient.invalidateQueries();
    } finally {
      setRetrying(false);
    }
  }

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-label={ok ? "Lista disponible" : "Sin acceso a la lista"}
        className="flex items-center gap-1.5 rounded-full bg-black/55 px-2.5 py-1 text-[11px] font-medium text-neutral-200 outline-none backdrop-blur transition active:scale-95"
      >
        <span className={`h-2 w-2 rounded-full ${ok ? "bg-emerald-400" : "bg-red-500"}`} />
        {ok ? "Lista disponible" : "Sin acceso"}
      </button>
      {open && (
        <div role="status" className="absolute right-0 top-full z-30 mt-2 w-72 rounded-xl border border-neutral-800 bg-neutral-900/95 p-3 text-sm shadow-2xl backdrop-blur">
          {ok ? (
            <p className="text-neutral-300">Tu lista responde: los canales, películas y series se pueden reproducir.</p>
          ) : (
            <ul className="flex flex-col gap-2">
              {down.map((p) => (
                <li key={p.provider_id}>
                  <p className="font-medium text-white">{p.name}</p>
                  <p className="text-neutral-400">{p.message ?? "No responde."}</p>
                </li>
              ))}
            </ul>
          )}
          <button
            type="button"
            onClick={retry}
            disabled={retrying}
            className="mt-3 w-full rounded-lg bg-neutral-800 px-3 py-2 text-sm font-medium text-white outline-none active:bg-neutral-700 disabled:opacity-50"
          >
            {retrying ? "Comprobando…" : "Comprobar otra vez"}
          </button>
        </div>
      )}
    </div>
  );
}
