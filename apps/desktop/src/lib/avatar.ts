import type { ProfileSummary } from "./api-types";

/** Colores de fondo del avatar — nada de subir una imagen, como los iconos por defecto
 * de Netflix: elegir de una paleta corta es instantáneo y no necesita almacenamiento de
 * ficheros. */
export const AVATAR_COLORS = ["#dc2626", "#ea580c", "#d97706", "#65a30d", "#059669", "#0891b2", "#4f46e5", "#a21caf"];

/** Iconos del avatar. Emoji, no imágenes: mismo motivo que los colores — cero
 * almacenamiento, se ven bien en cualquier tamaño, y dan variedad real sin tener que
 * dibujar ni empaquetar ningún icono propio. */
export const AVATAR_ICONS = ["🎬", "🍿", "📺", "🎮", "🦸", "🐱", "🐶", "🦄", "⚽", "🎵", "🌟", "🍕", "🚀", "👑", "🎨", "😀"];

/** `avatar` guarda `"<icono>|<colorHex>"`, o solo el color (formato antiguo, de antes
 * de que hubiera iconos) — se interpreta como "sin icono, solo color" para no romper
 * perfiles ya creados. Compartido entre `ProfilesScreen` (crear/editar) y
 * `ProfileSwitcher` (mostrar el activo) para no tener dos copias de este parseo. */
export function parseAvatar(profile: Pick<ProfileSummary, "avatar" | "name">): { icon: string | null; color: string } {
  const raw = profile.avatar ?? "";
  const [maybeIcon, maybeColor] = raw.split("|");
  if (maybeColor && AVATAR_COLORS.includes(maybeColor)) {
    return { icon: maybeIcon || null, color: maybeColor };
  }
  if (maybeIcon && AVATAR_COLORS.includes(maybeIcon)) {
    return { icon: null, color: maybeIcon }; // formato antiguo: solo color, sin "|".
  }
  // Sin avatar guardado (o algo irreconocible): color determinista a partir del
  // nombre, para que no cambie en cada render.
  const index = profile.name.charCodeAt(0) % AVATAR_COLORS.length;
  return { icon: null, color: AVATAR_COLORS[index] ?? "#dc2626" };
}

export function formatAvatar(icon: string | null, color: string): string {
  return icon ? `${icon}|${color}` : color;
}
