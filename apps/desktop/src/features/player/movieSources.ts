import { apiPost } from "../../lib/api-client";
import type { SlowReport } from "../../lib/api-types";

/** Una fuente de película (`MovieSourceSummary`) o de episodio (`EpisodeSourceSummary`). El
 * núcleo ya las da ordenadas: la que mejor ha ido, primero (`catalog::sources_rank`). */
export type PlaySource = { id: number; quality: string | null; language: string | null };

/** Nombre de una fuente en el reproductor: «Fuente 2 · 1080p · ES» (o solo «Fuente 2»). */
export function sourceLabel(source: PlaySource, index: number): string {
  const details = [source.quality, source.language].filter((part): part is string => Boolean(part));
  return details.length > 0 ? `Fuente ${index + 1} · ${details.join(" · ")}` : `Fuente ${index + 1}`;
}

/** Posición de la siguiente fuente sin probar tras un fallo, o -1 si ya se probaron todas.
 * Cada fuente se prueba una vez: si todas fallan, se enseña el error de verdad. */
export function nextUntriedSource(sources: PlaySource[], tried: ReadonlySet<number>): number {
  return sources.findIndex((source) => !tried.has(source.id));
}

/** Aviso al cambiar solo de fuente. */
export function failoverNotice(fromIndex: number, toIndex: number): string {
  return `La fuente ${fromIndex + 1} no responde: probando la ${toIndex + 1}`;
}

/** Aviso al cambiar solo de fuente porque iba lenta (con «Volver» al lado). */
export function slowNotice(fromIndex: number, toIndex: number): string {
  return `La fuente ${fromIndex + 1} va lenta: cambiando a la ${toIndex + 1}`;
}

// «Va lenta» (F17): la misma regla en el reproductor del PC y en el del iPhone, para que
// cambien de fuente en los mismos casos. Como mucho un cambio por lentitud en cada
// reproducción, y ninguno si el usuario eligió fuente a mano.

/** Sin empezar a verse en este tiempo, la fuente va lenta. Generoso: hay paneles que
 * tardan sin estar mal, y cambiar cuesta otra conexión con el proveedor. */
export const SLOW_START_MS = 20_000;
/** Un parón más corto que esto es un tirón y no cuenta. */
export const MIN_STALL_MS = 1_500;
/** Un solo parón así de largo ya es ir lenta. */
export const LONG_STALL_MS = 10_000;
/** Tantos parones en la ventana también. */
export const STALLS_TO_SWITCH = 3;
export const STALL_WINDOW_MS = 120_000;

/** Un parón: cuándo acabó (ms de reloj) y cuánto duró. */
export type Stall = { at: number; ms: number };

/** Si los parones recientes dicen que esta fuente va lenta. */
export function stallsSaySlow(stalls: readonly Stall[], now: number): boolean {
  const recent = stalls.filter((stall) => stall.ms >= MIN_STALL_MS && now - stall.at <= STALL_WINDOW_MS);
  return recent.some((stall) => stall.ms >= LONG_STALL_MS) || recent.length >= STALLS_TO_SWITCH;
}

/** Los parones que cuentan, para el diagnóstico: cuántos y el peor. */
export function stallSummary(stalls: readonly Stall[], now: number): { stalls: number; waitedMs: number } {
  const recent = stalls.filter((stall) => stall.ms >= MIN_STALL_MS && now - stall.at <= STALL_WINDOW_MS);
  return { stalls: recent.length, waitedMs: Math.max(0, ...recent.map((stall) => Math.round(stall.ms))) };
}

/** Avisa al núcleo de que una fuente va lenta: baja en el orden y queda en el diagnóstico. */
export function reportSlowSource(report: SlowReport): void {
  apiPost("/api/playback/slow", report).catch(() => {});
}
