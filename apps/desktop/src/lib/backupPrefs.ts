import type { ProfileSummary } from "./api-types";

/** Preferencias que viven en este aparato (no en el núcleo) y van también en la copia de
 * seguridad: subtítulos, reproducción y últimas pistas elegidas. Se guardan por nombre de
 * perfil, porque el id cambia de un aparato a otro. */
const KEYS = ["subtitles", "playback", "tracks"] as const;

type UiBackup = Record<string, Partial<Record<(typeof KEYS)[number], unknown>>>;

/** Las preferencias de cada perfil, para meterlas en la copia. */
export function collectUiBackup(profiles: ProfileSummary[]): UiBackup {
  const out: UiBackup = {};
  for (const profile of profiles) {
    const prefs: UiBackup[string] = {};
    for (const key of KEYS) {
      try {
        const raw = window.localStorage.getItem(`vibe.${key}.${profile.id}`);
        if (raw) prefs[key] = JSON.parse(raw) as unknown;
      } catch {
        // Sin almacenamiento o valor roto: esa preferencia no va en la copia.
      }
    }
    if (Object.keys(prefs).length > 0) out[profile.name] = prefs;
  }
  return out;
}

/** Reparte las preferencias de una copia entre los perfiles de aquí con el mismo nombre. */
export function applyUiBackup(contents: string, profiles: ProfileSummary[]) {
  let ui: unknown;
  try {
    ui = (JSON.parse(contents) as { ui?: unknown }).ui;
  } catch {
    return;
  }
  if (!ui || typeof ui !== "object") return;
  for (const [name, prefs] of Object.entries(ui as UiBackup)) {
    const profile = profiles.find((p) => p.name.trim().toLowerCase() === name.trim().toLowerCase());
    if (!profile || !prefs || typeof prefs !== "object") continue;
    for (const key of KEYS) {
      if (prefs[key] === undefined) continue;
      try {
        window.localStorage.setItem(`vibe.${key}.${profile.id}`, JSON.stringify(prefs[key]));
      } catch {
        // Sin almacenamiento: no se puede guardar, no pasa nada.
      }
    }
  }
  // Que Ajustes y el reproductor vuelvan a leerlas.
  window.dispatchEvent(new Event("vibe-subtitle-style"));
  window.dispatchEvent(new Event("vibe-playback-prefs"));
}
