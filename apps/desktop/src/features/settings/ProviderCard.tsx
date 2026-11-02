import { useState } from "react";
import type { ProviderSummary, SyncProgress } from "../../lib/api-types";
import { EyeIcon } from "../../components/icons";
import { BUTTON, BUTTON_DANGER } from "./styles";

export type ProviderSyncState = { running: boolean; message: string | null; failed: boolean };

/** A partir de cuántos días antes de caducar se avisa en ámbar. */
const EXPIRY_WARNING_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;

type Props = {
  provider: ProviderSummary;
  progress?: SyncProgress;
  sync?: ProviderSyncState;
  onSync: () => void;
  onDelete: () => void;
  deleting: boolean;
  deleteError: string | null;
};

/** Una lista IPTV de un vistazo: si está activa o caduca, cuándo se sincronizó y con
 * cuántas conexiones. La dirección y el usuario salen ocultos (por si se comparte pantalla)
 * y se enseñan con el ojo. Borrar pide un segundo toque: se lleva todo su catálogo. */
export function ProviderCard({ provider, progress, sync, onSync, onDelete, deleting, deleteError }: Props) {
  const [reveal, setReveal] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const status = expiryStatus(provider.expires_at);
  const address = `${provider.kind === "xtream" ? "Xtream Codes" : "Lista M3U"} · ${reveal ? provider.base_url_display : "••••••••••••"}${provider.username ? ` · ${reveal ? provider.username : "•••••"}` : ""}`;

  return (
    <article className="flex flex-col gap-4 rounded-2xl bg-white/[0.04] p-4 ring-1 ring-white/[0.06] sm:p-5 ios:rounded-[22px] ios:bg-ios-surface-1 ios:ring-0">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className={`h-2 w-2 shrink-0 rounded-full ${status.dot}`} aria-hidden />
            <h3 className="truncate text-[17px] font-semibold text-white">{provider.name}</h3>
          </div>
          <p className="flex items-center gap-2 break-all text-[13px] text-neutral-400">
            <span>{address}</span>
            <button
              type="button"
              onClick={() => setReveal((r) => !r)}
              aria-label={reveal ? "Ocultar dirección y usuario" : "Ver dirección y usuario"}
              aria-pressed={reveal}
              className="shrink-0 rounded p-1 text-neutral-500 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white/70"
            >
              <EyeIcon className="h-4 w-4" />
            </button>
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {confirming ? (
            <>
              <span className="self-center text-sm text-neutral-300">Se borrará todo su catálogo.</span>
              <button
                type="button"
                className="rounded-lg bg-red-700 px-3.5 py-2 text-sm font-semibold text-white outline-none hover:bg-red-600 focus-visible:ring-2 focus-visible:ring-white/70 disabled:opacity-50 ios:rounded-full"
                onClick={() => {
                  setConfirming(false);
                  onDelete();
                }}
                disabled={deleting}
              >
                Sí, eliminar
              </button>
              <button type="button" className={BUTTON} onClick={() => setConfirming(false)}>
                Cancelar
              </button>
            </>
          ) : (
            <>
              <button type="button" className={BUTTON} onClick={onSync} disabled={sync?.running === true}>
                {sync?.running ? "Sincronizando…" : "Sincronizar"}
              </button>
              <button type="button" className={BUTTON_DANGER} onClick={() => setConfirming(true)} disabled={deleting}>
                Eliminar
              </button>
            </>
          )}
        </div>
      </div>

      <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Fact label="Estado" value={status.label} tone={status.tone} />
        <Fact label="Conexiones a la vez" value={String(provider.max_connections)} />
        <Fact label="Última sincronización" value={provider.last_sync_at ? relativeTime(provider.last_sync_at) : "Nunca"} />
      </dl>

      {progress && progress.total > 0 && progress.current < progress.total && (
        <div className="flex flex-col gap-1.5">
          <p className="text-[13px] text-neutral-400">
            {progress.stage}: {progress.current.toLocaleString()} de {progress.total.toLocaleString()}
          </p>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
            <div className="h-full rounded-full bg-red-600 transition-[width]" style={{ width: `${Math.min(100, (progress.current / progress.total) * 100)}%` }} />
          </div>
        </div>
      )}
      {sync?.message && (
        <p role="status" className={`text-[13px] ${sync.failed ? "text-red-400" : "text-neutral-400"}`}>
          {sync.message}
        </p>
      )}
      {deleteError && <p className="text-[13px] text-red-400">{deleteError}</p>}
    </article>
  );
}

function Fact({ label, value, tone = "text-white" }: { label: string; value: string; tone?: string }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-xl bg-black/20 px-3 py-2.5 ios:bg-ios-surface-2">
      <dt className="text-[12px] text-neutral-500">{label}</dt>
      <dd className={`text-sm font-medium ${tone}`}>{value}</dd>
    </div>
  );
}

function expiryStatus(expiresAt: number | null): { label: string; tone: string; dot: string } {
  if (expiresAt == null) return { label: "Activa", tone: "text-emerald-400", dot: "bg-emerald-500" };
  const daysLeft = Math.floor((expiresAt * 1000 - Date.now()) / DAY_MS);
  const date = new Date(expiresAt * 1000).toLocaleDateString();
  if (daysLeft < 0) return { label: `Caducó el ${date}`, tone: "text-red-400", dot: "bg-red-500" };
  if (daysLeft <= EXPIRY_WARNING_DAYS) return { label: `Caduca en ${daysLeft} día${daysLeft === 1 ? "" : "s"}`, tone: "text-amber-400", dot: "bg-amber-400" };
  return { label: `Activa hasta el ${date}`, tone: "text-emerald-400", dot: "bg-emerald-500" };
}

function relativeTime(epochSeconds: number): string {
  const minutes = Math.round((Date.now() - epochSeconds * 1000) / 60_000);
  if (minutes < 1) return "Ahora mismo";
  if (minutes < 60) return `Hace ${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `Hace ${hours} h`;
  const days = Math.round(hours / 24);
  return days === 1 ? "Ayer" : `Hace ${days} días`;
}
