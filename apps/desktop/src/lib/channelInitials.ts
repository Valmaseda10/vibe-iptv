/** Palabras que no cuentan para las iniciales de un canal. */
const SMALL_WORDS = new Set(["de", "del", "la", "las", "el", "los", "y", "en", "por"]);
const QUALITY = /\b(HD|FHD|UHD|4K|SD|1080p?|720p?)\b/gi;

/** Iniciales de un canal sin logo: «M+ Liga de Campeones 2» → «M+LC2»; «La 2» → «L2». */
export function channelInitials(name: string): string {
  const words = name
    .replace(QUALITY, "")
    .split(/\s+/)
    .filter((w) => w && !SMALL_WORDS.has(w.toLowerCase()));
  const out = words.map((w) => (/^[0-9]+$/.test(w) || (/^[A-Z0-9+]+$/.test(w) && w.length <= 3) ? w : w.charAt(0).toUpperCase())).join("");
  return out.slice(0, 6) || "TV";
}
