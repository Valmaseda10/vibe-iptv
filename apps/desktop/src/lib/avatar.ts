import type { ProfileSummary } from "./api-types";

/** Colores lisos de fondo (los de siempre). */
export const AVATAR_COLORS = ["#dc2626", "#ea580c", "#d97706", "#65a30d", "#059669", "#0891b2", "#4f46e5", "#a21caf"];

/** Degradados de fondo (2026-11-02), con la clave que se guarda (`g:<id>`). */
export const AVATAR_GRADIENTS: readonly { id: string; css: string }[] = [
  { id: "g:vibe", css: "linear-gradient(135deg,#ff8a3d,#ff2d55 55%,#c2187a)" },
  { id: "g:ocean", css: "linear-gradient(135deg,#22d3ee,#2563eb 60%,#4338ca)" },
  { id: "g:forest", css: "linear-gradient(135deg,#a3e635,#16a34a 55%,#065f46)" },
  { id: "g:sunset", css: "linear-gradient(135deg,#fde047,#f97316 55%,#be123c)" },
  { id: "g:night", css: "linear-gradient(135deg,#6366f1,#7c3aed 50%,#1e1b4b)" },
  { id: "g:candy", css: "linear-gradient(135deg,#f9a8d4,#ec4899 55%,#9333ea)" },
  { id: "g:steel", css: "linear-gradient(135deg,#94a3b8,#475569 55%,#0f172a)" },
  { id: "g:gold", css: "linear-gradient(135deg,#fef08a,#eab308 50%,#a16207)" },
];

/** Iconos emoji de antes (perfiles ya creados y los que los prefieran). */
export const AVATAR_ICONS = ["🎬", "🍿", "📺", "🎮", "🦸", "🐱", "🐶", "🦄", "⚽", "🎵", "🌟", "🍕", "🚀", "👑", "🎨", "😀"];

/** Estilos de personajes ilustrados (DiceBear, diseños CC0: dominio público). */
export const CHARACTER_STYLES = [
  { id: "lorelei", label: "Retratos" },
  { id: "notionists", label: "Dibujos" },
  { id: "open-peeps", label: "Gente" },
] as const;
export type CharacterStyle = (typeof CHARACTER_STYLES)[number]["id"];

/** Lo que dice el campo `avatar` de un perfil. Formatos (todos viajan con la cuenta):
 * - `"#hex"` o `"<emoji>|#hex"`: los de siempre (inicial o emoji sobre un color).
 * - `"fluent:<slug>|<fondo>"`: un icono 3D (Microsoft Fluent Emoji, MIT).
 * - `"char:<estilo>:<semilla>|<fondo>"`: un personaje ilustrado generado aquí.
 * - `"tmdb:<ruta>"`: la imagen de una película o serie del catálogo.
 * `<fondo>` es un color de `AVATAR_COLORS` o un degradado `g:<id>`. Lo que no se entienda
 * (una versión más nueva) se enseña como la inicial sobre un color. */
export type Avatar =
  | { kind: "initial"; bg: string }
  | { kind: "emoji"; emoji: string; bg: string }
  | { kind: "fluent"; slug: string; bg: string }
  | { kind: "character"; style: CharacterStyle; seed: string; bg: string }
  | { kind: "title"; path: string };

const isBackground = (value: string | undefined): value is string =>
  value !== undefined && (AVATAR_COLORS.includes(value) || AVATAR_GRADIENTS.some((g) => g.id === value));

function fallbackColor(name: string): string {
  return AVATAR_COLORS[name.charCodeAt(0) % AVATAR_COLORS.length] ?? "#dc2626";
}

export function parseAvatar(profile: Pick<ProfileSummary, "avatar" | "name">): Avatar {
  const raw = profile.avatar ?? "";
  const fallback: Avatar = { kind: "initial", bg: fallbackColor(profile.name) };
  if (raw.startsWith("tmdb:")) return raw.length > 5 ? { kind: "title", path: raw.slice(5) } : fallback;
  const [main = "", background] = raw.split("|");
  const bg = isBackground(background) ? background : fallbackColor(profile.name);
  if (main.startsWith("fluent:")) return { kind: "fluent", slug: main.slice(7), bg };
  if (main.startsWith("char:")) {
    const [, style, ...seed] = main.split(":");
    if (CHARACTER_STYLES.some((s) => s.id === style) && seed.length > 0) return { kind: "character", style: style as CharacterStyle, seed: seed.join(":"), bg };
    return fallback;
  }
  if (isBackground(background) && main) return { kind: "emoji", emoji: main, bg };
  // Formato antiguo: solo el color, sin «|».
  if (isBackground(main)) return { kind: "initial", bg: main };
  return fallback;
}

export function formatAvatar(avatar: Avatar): string {
  switch (avatar.kind) {
    case "initial":
      return avatar.bg;
    case "emoji":
      return `${avatar.emoji}|${avatar.bg}`;
    case "fluent":
      return `fluent:${avatar.slug}|${avatar.bg}`;
    case "character":
      return `char:${avatar.style}:${avatar.seed}|${avatar.bg}`;
    case "title":
      return `tmdb:${avatar.path}`;
  }
}

/** El fondo de un avatar como CSS (color o degradado). */
export function backgroundCss(bg: string): string {
  return AVATAR_GRADIENTS.find((g) => g.id === bg)?.css ?? bg;
}
