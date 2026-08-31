import { useSyncStatus, syncStageLabel } from "../lib/syncStatus";

/** Aviso discreto, abajo a la izquierda, de que hay una sincronización en marcha o que
 * acaba de terminar — visible desde cualquier pantalla (ver `SyncStatusProvider`). No
 * bloquea nada ni pide atención: solo confirma que algo está pasando, para que una
 * sincronización de 30 s en segundo plano no parezca que la app no ha hecho nada. */
export function SyncStatusBanner() {
  const { active, justFinished } = useSyncStatus();

  if (!active && !justFinished) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none fixed bottom-4 left-4 z-30 flex items-center gap-2 rounded-lg border border-neutral-800 bg-neutral-900/95 px-3 py-2 text-xs text-neutral-300 shadow-lg backdrop-blur"
    >
      {active ? (
        <>
          <span className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-neutral-600 border-t-red-600" />
          <span>
            Sincronizando {syncStageLabel(active.stage)}
            {active.total > 0 ? ` (${active.current}/${active.total})` : "…"}
          </span>
        </>
      ) : (
        <>
          <span className="h-2 w-2 shrink-0 rounded-full bg-emerald-500" />
          <span>Catálogo actualizado</span>
        </>
      )}
    </div>
  );
}
