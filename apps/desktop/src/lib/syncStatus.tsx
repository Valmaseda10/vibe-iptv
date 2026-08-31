import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import type { SyncProgress } from "./api-types";

/** Ninguna de estas necesita coincidir exacta: TanStack Query invalida por prefijo, así
 * que `["movies"]` también alcanza a `["movies", profileId]`. Es la lista de todo lo
 * que puede quedarse desactualizado tras una sincronización que no pasó por Ajustes
 * (arranque automático, o Ajustes ya cerrado cuando termina). */
const CATALOG_QUERY_KEYS = ["providers", "home", "movies", "series", "live-categories", "live-channels", "favorites", "search"];

/** Cuánto se espera sin ningún evento nuevo antes de dar la sincronización por
 * terminada. No hay un evento explícito de "sincronización completa" — solo una
 * cadena de progresos por etapa — así que un hueco de silencio es la señal más
 * sencilla y a la vez robusta a que cambie el orden o el número de etapas. */
const INACTIVITY_MS = 3000;
const CONFIRMATION_VISIBLE_MS = 4000;

type SyncStatusValue = {
  /** El último progreso recibido, mientras se considera que la sincronización sigue en
   * marcha (silencio menor de `INACTIVITY_MS`). `null` en reposo. */
  active: SyncProgress | null;
  /** Se pone a `true` un momento justo después de que `active` pase a `null` — para
   * poder enseñar una confirmación breve en vez de que el aviso desaparezca de golpe. */
  justFinished: boolean;
};

const SyncStatusContext = createContext<SyncStatusValue>({ active: null, justFinished: false });

/** Se monta una sola vez en la raíz de la app (`main.tsx`): así el aviso de
 * sincronización se ve desde cualquier pantalla, no solo si estás dentro de Ajustes —
 * antes, salir de Ajustes durante una sincronización la volvía completamente invisible. */
export function SyncStatusProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [active, setActive] = useState<SyncProgress | null>(null);
  const [justFinished, setJustFinished] = useState(false);
  const inactivityTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const confirmationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const unlistenPromise = listen<SyncProgress>("sync://progress", (event) => {
      setActive(event.payload);
      setJustFinished(false);
      if (inactivityTimer.current) clearTimeout(inactivityTimer.current);
      if (confirmationTimer.current) clearTimeout(confirmationTimer.current);

      inactivityTimer.current = setTimeout(() => {
        setActive(null);
        setJustFinished(true);
        // El catálogo (y la lista de proveedores, por `last_sync_at`) pudo cambiar
        // mientras se miraba otra pantalla — sin esto, habría que navegar fuera y
        // volver para que TanStack Query decidiera refrescarlo por sí solo.
        for (const key of CATALOG_QUERY_KEYS) queryClient.invalidateQueries({ queryKey: [key] });
        confirmationTimer.current = setTimeout(() => setJustFinished(false), CONFIRMATION_VISIBLE_MS);
      }, INACTIVITY_MS);
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
      if (inactivityTimer.current) clearTimeout(inactivityTimer.current);
      if (confirmationTimer.current) clearTimeout(confirmationTimer.current);
    };
  }, [queryClient]);

  return <SyncStatusContext.Provider value={{ active, justFinished }}>{children}</SyncStatusContext.Provider>;
}

export function useSyncStatus() {
  return useContext(SyncStatusContext);
}

/** Texto legible de la etapa en curso — los nombres de `SyncProgress.stage` son los que
 * usa `catalog::sync` internamente (`"categorias"`, `"canales"`...), no pensados para
 * enseñarse tal cual. */
export function syncStageLabel(stage: string): string {
  switch (stage) {
    case "categorias":
      return "categorías";
    case "canales":
      return "canales";
    case "peliculas":
      return "películas";
    case "series":
      return "series";
    case "tmdb":
      return "metadatos (TMDB)";
    default:
      return stage;
  }
}
