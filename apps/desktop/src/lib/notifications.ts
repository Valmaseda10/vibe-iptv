import { invoke } from "@tauri-apps/api/core";
import type { ScheduledNotice } from "./api-types";

/** Avisos del sistema (F17), en el PC y en el iPhone con la misma interfaz: en el iPhone
 * quedan programados (avisan con la app cerrada); en el PC se enseñan al llegar la hora,
 * con la app abierta (Windows no los programa). Ver `src-tauri/src/notifications.rs`. */

export type NotificationPermission = "granted" | "denied" | "prompt";

export async function notificationPermission(): Promise<NotificationPermission> {
  try {
    return (await invoke<string>("notifications_permission")) as NotificationPermission;
  } catch {
    return "denied";
  }
}

/** Pide permiso si aún no se ha preguntado (en el iPhone sale la pregunta del sistema). */
export async function ensureNotificationPermission(): Promise<boolean> {
  const current = await notificationPermission();
  if (current === "granted") return true;
  if (current === "denied") return false;
  try {
    return (await invoke<string>("notifications_request")) === "granted";
  } catch {
    return false;
  }
}

export async function showNotification(title: string, body: string): Promise<void> {
  if ((await notificationPermission()) !== "granted") return;
  await invoke("notifications_show", { title, body }).catch(() => {});
}

let canScheduleCache: Promise<boolean> | null = null;
/** Si este aparato deja los avisos programados (iPhone). */
export function canScheduleNotifications(): Promise<boolean> {
  canScheduleCache ??= invoke<boolean>("notifications_can_schedule").catch(() => false);
  return canScheduleCache;
}

/** Cambia todos los avisos programados por estos (solo en el iPhone). */
export async function replaceScheduledNotifications(notices: ScheduledNotice[]): Promise<void> {
  if ((await notificationPermission()) !== "granted") return;
  await invoke("notifications_replace_scheduled", { notices }).catch(() => {});
}

/** Id estable de un aviso a partir de su clave (FNV-1a de 31 bits). */
export function noticeId(key: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash & 0x7fffffff;
}

const SHOWN_KEY = "vibe.notices.shown";

/** Si este aviso ya se enseñó en este aparato (PC): para no repetirlo al refrescar. Es
 * una comodidad de este aparato; si el navegador no deja guardar, se repite y ya está. */
export function markShownOnce(key: string): boolean {
  try {
    const shown: Record<string, number> = JSON.parse(localStorage.getItem(SHOWN_KEY) ?? "{}");
    if (shown[key]) return false;
    const now = Date.now();
    shown[key] = now;
    // Lo de hace más de dos días ya no hace falta recordarlo.
    for (const [k, at] of Object.entries(shown)) if (now - at > 2 * 86_400_000) delete shown[k];
    localStorage.setItem(SHOWN_KEY, JSON.stringify(shown));
    return true;
  } catch {
    return true;
  }
}
