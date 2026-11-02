import { createAvatar } from "@dicebear/core";
import * as lorelei from "@dicebear/lorelei";
import * as notionists from "@dicebear/notionists";
import * as openPeeps from "@dicebear/open-peeps";
import type { CharacterStyle } from "./avatar";

/** Los iconos 3D empaquetados con la app (`assets/avatars/3d`), por nombre. */
const FLUENT_FILES = import.meta.glob<string>("../assets/avatars/3d/*.webp", { eager: true, query: "?url", import: "default" });
const FLUENT_URLS: Record<string, string> = Object.fromEntries(
  Object.entries(FLUENT_FILES).map(([path, url]) => [path.split("/").pop()?.replace(".webp", "") ?? path, url]),
);

export function fluentUrl(slug: string): string | null {
  return FLUENT_URLS[slug] ?? null;
}

const characterCache = new Map<string, string>();

/** El personaje ilustrado de una semilla, como imagen (SVG). Se genera aquí, sin red, y
 * siempre igual para la misma semilla: así se ve igual en todos los aparatos. */
export function characterUrl(style: CharacterStyle, seed: string): string {
  const key = `${style}:${seed}`;
  const cached = characterCache.get(key);
  if (cached) return cached;
  const options = { seed, backgroundColor: ["transparent"] };
  const url = (
    style === "lorelei" ? createAvatar(lorelei, options) : style === "notionists" ? createAvatar(notionists, options) : createAvatar(openPeeps, options)
  ).toDataUri();
  characterCache.set(key, url);
  return url;
}
