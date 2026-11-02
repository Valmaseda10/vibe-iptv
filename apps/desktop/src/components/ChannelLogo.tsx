import { useState } from "react";

/** Logo de un canal (del proveedor). Si no hay o no carga (muchos proveedores apuntan a
 * imágenes que ya no existen), las iniciales del canal en vez de la imagen rota. */
export function ChannelLogo({ url, name, className = "h-5 w-5" }: { url: string | null; name?: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  const initials = name ? initialsOf(name) : "TV";
  return (
    <span className={`flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-neutral-900 ${className}`}>
      {url && !failed ? (
        <img src={url} alt="" onError={() => setFailed(true)} className="h-full w-full object-contain" />
      ) : (
        <span className="px-1 text-center text-[0.65em] font-bold leading-none tracking-tight text-neutral-400">{initials}</span>
      )}
    </span>
  );
}

/** «M+ Liga de Campeones 2» → «M+LC2»; «La 2» → «L2»: corto y reconocible. */
const SMALL_WORDS = new Set(["de", "del", "la", "las", "el", "los", "y", "en", "por"]);
function initialsOf(name: string): string {
  const words = name
    .replace(/\b(HD|FHD|UHD|4K|SD|1080p?|720p?)\b/gi, "")
    .split(/\s+/)
    .filter((w) => w && !SMALL_WORDS.has(w.toLowerCase()));
  const out = words.map((w) => (/^[0-9]+$/.test(w) || (/^[A-Z0-9+]+$/.test(w) && w.length <= 3) ? w : w.charAt(0).toUpperCase())).join("");
  return out.slice(0, 6) || "TV";
}
