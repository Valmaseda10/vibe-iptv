import type { MovieSourceSummary } from "../../lib/api-types";

/** Nombre de una fuente en el reproductor: «Fuente 2 · 1080p · ES» (o solo «Fuente 2»). */
export function sourceLabel(source: MovieSourceSummary, index: number): string {
  const details = [source.quality, source.language].filter((part): part is string => Boolean(part));
  return details.length > 0 ? `Fuente ${index + 1} · ${details.join(" · ")}` : `Fuente ${index + 1}`;
}

/** Posición de la siguiente fuente sin probar tras un fallo, o -1 si ya se probaron todas.
 * Cada fuente se prueba una vez: si todas fallan, se enseña el error de verdad. */
export function nextUntriedSource(sources: MovieSourceSummary[], tried: ReadonlySet<number>): number {
  return sources.findIndex((source) => !tried.has(source.id));
}

/** Aviso al cambiar solo de fuente. */
export function failoverNotice(fromIndex: number, toIndex: number): string {
  return `La fuente ${fromIndex + 1} no responde: probando la ${toIndex + 1}`;
}
