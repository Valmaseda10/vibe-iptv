import { useEffect, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { IS_IOS } from "../lib/platform";
import { CloseIcon } from "./icons";

type UpdateReady = { version: string; notes: string | null };

/** «Hay una versión nueva»: el núcleo la descarga solo, sin nada reproduciéndose
 * (`updater.rs`), y aquí se ofrece reiniciar. Nunca encima del reproductor; «Más tarde» la
 * deja para el próximo arranque. Solo en el PC. */
export function UpdateToast() {
  const [ready, setReady] = useState<UpdateReady | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inPlayer = useRouterState({ select: (state) => state.location.pathname.startsWith("/player/") });

  useEffect(() => {
    if (IS_IOS) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    invoke<UpdateReady | null>("update_ready")
      .then((value) => {
        if (!cancelled && value) setReady(value);
      })
      .catch(() => {});
    listen<UpdateReady>("updater://ready", (event) => setReady(event.payload))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  if (!ready || dismissed || inPlayer) return null;

  async function install() {
    setInstalling(true);
    setError(null);
    try {
      await invoke("install_update");
    } catch (err) {
      setError(typeof err === "string" ? err : "No se pudo instalar la versión nueva.");
      setInstalling(false);
    }
  }

  return (
    <div
      role="status"
      className="preview-in fixed bottom-6 left-6 z-[45] flex max-w-sm items-start gap-3 rounded-2xl bg-neutral-900/95 p-4 shadow-2xl shadow-black/60 ring-1 ring-white/10 backdrop-blur"
    >
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-white">Versión {ready.version} lista</p>
        <p className="mt-0.5 text-xs text-neutral-400">{error ?? "Se instala en unos segundos y la app vuelve a abrirse."}</p>
        <div className="mt-3 flex gap-2">
          <button
            type="button"
            onClick={() => void install()}
            disabled={installing}
            className="rounded-full bg-white px-4 py-1.5 text-xs font-semibold text-black outline-none transition hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-red-600 disabled:opacity-60"
          >
            {installing ? "Instalando…" : "Reiniciar y actualizar"}
          </button>
          <button
            type="button"
            onClick={() => setDismissed(true)}
            className="rounded-full bg-white/10 px-4 py-1.5 text-xs font-medium text-white outline-none transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white"
          >
            Más tarde
          </button>
        </div>
      </div>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        className="rounded-full p-1 text-neutral-400 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white"
        aria-label="Cerrar"
      >
        <CloseIcon className="h-4 w-4" />
      </button>
    </div>
  );
}
