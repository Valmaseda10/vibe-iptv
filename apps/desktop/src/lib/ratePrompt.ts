import { useSyncExternalStore } from "react";

/** Título por el que preguntar «¿Te ha gustado?» al terminar de verlo. */
export type RateCandidate = {
  profileId: number;
  kind: "movie" | "series";
  id: number;
  title: string;
  posterPath: string | null;
};

/** A partir de aquí, algo se ha visto entero (como el núcleo, que lo da por visto al 90 %). */
export const WATCHED_RATIO = 0.9;

let current: RateCandidate | null = null;
const listeners = new Set<() => void>();

function askedKey(candidate: RateCandidate): string {
  return `vibe.rateAsked.${candidate.profileId}.${candidate.kind}.${candidate.id}`;
}

function wasAsked(candidate: RateCandidate): boolean {
  try {
    return window.localStorage.getItem(askedKey(candidate)) === "1";
  } catch {
    return false;
  }
}

/** Pide preguntar por `candidate` (una vez por título y perfil en este aparato). */
export function askToRate(candidate: RateCandidate | null): void {
  if (!candidate || !candidate.title || wasAsked(candidate)) return;
  current = candidate;
  listeners.forEach((listener) => listener());
}

/** Ya se preguntó (se contestara o no): no se vuelve a preguntar por este título. */
export function dismissRatePrompt(): void {
  if (current) {
    try {
      window.localStorage.setItem(askedKey(current), "1");
    } catch {
      // Sin almacenamiento: podría volver a preguntar otro día.
    }
  }
  current = null;
  listeners.forEach((listener) => listener());
}

export function useRateCandidate(): RateCandidate | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
  );
}
