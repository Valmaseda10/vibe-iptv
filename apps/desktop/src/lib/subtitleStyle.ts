import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { useActiveProfile } from "./activeProfile";
import type { SubtitleStyle } from "./api-types";

/** Aspecto de los subtítulos por perfil, guardado en este aparato (como las pistas de audio
 * y subtítulos preferidas). El mismo en el reproductor del PC (`::cue`) y en el de iPhone y
 * iPad (VLC, ver `VibePlayer.swift`). */
export type { SubtitleStyle };

export const DEFAULT_SUBTITLE_STYLE: SubtitleStyle = { size: "normal", look: "outline", color: "white", font: "system" };

export const SUBTITLE_SIZES = [
  { value: "small", label: "Pequeño" },
  { value: "normal", label: "Normal" },
  { value: "large", label: "Grande" },
  { value: "xlarge", label: "Muy grande" },
] as const;
export const SUBTITLE_LOOKS = [
  { value: "outline", label: "Contorno" },
  { value: "box", label: "Con fondo" },
  { value: "shadow", label: "Sombra" },
] as const;
export const SUBTITLE_COLORS = [
  { value: "white", label: "Blanco" },
  { value: "yellow", label: "Amarillo" },
] as const;
export const SUBTITLE_FONTS = [
  { value: "system", label: "Normal" },
  { value: "serif", label: "Con serifa" },
  { value: "wide", label: "Ancha" },
] as const;

const SIZE_SCALE: Record<string, number> = { small: 0.8, normal: 1, large: 1.25, xlarge: 1.55 };
const COLOR_HEX: Record<string, string> = { white: "#ffffff", yellow: "#ffd60a" };
const FONT_FAMILY: Record<string, string> = {
  system: 'system-ui, -apple-system, "Segoe UI", sans-serif',
  serif: 'Georgia, "Times New Roman", serif',
  wide: "Verdana, Geneva, sans-serif",
};
const OUTLINE_SHADOW = "-1.5px -1.5px 0 #000, 1.5px -1.5px 0 #000, -1.5px 1.5px 0 #000, 1.5px 1.5px 0 #000";

const CHANGED_EVENT = "vibe-subtitle-style";
const storageKey = (profileId: number | null) => `vibe.subtitles.${profileId ?? "sin-perfil"}`;

function valid(style: Partial<SubtitleStyle> | null): SubtitleStyle {
  const pick = <T extends string>(value: unknown, options: readonly { value: T }[], fallback: T): T =>
    options.some((o) => o.value === value) ? (value as T) : fallback;
  return {
    size: pick(style?.size, SUBTITLE_SIZES, "normal"),
    look: pick(style?.look, SUBTITLE_LOOKS, "outline"),
    color: pick(style?.color, SUBTITLE_COLORS, "white"),
    font: pick(style?.font, SUBTITLE_FONTS, "system"),
  };
}

export function readSubtitleStyle(profileId: number | null): SubtitleStyle {
  try {
    const raw = window.localStorage.getItem(storageKey(profileId));
    return valid(raw ? (JSON.parse(raw) as Partial<SubtitleStyle>) : null);
  } catch {
    return DEFAULT_SUBTITLE_STYLE;
  }
}

export function saveSubtitleStyle(profileId: number | null, style: SubtitleStyle) {
  try {
    window.localStorage.setItem(storageKey(profileId), JSON.stringify(valid(style)));
  } catch {
    // Sin almacenamiento: vale para esta sesión, no se recuerda.
  }
  window.dispatchEvent(new Event(CHANGED_EVENT));
}

/** El aspecto del perfil activo; se pone al día si cambia en otra pantalla (Ajustes, el
 * reproductor). */
export function useSubtitleStyle(): [SubtitleStyle, (next: SubtitleStyle) => void] {
  const { profileId } = useActiveProfile();
  const [style, setStyle] = useState(() => readSubtitleStyle(profileId));
  useEffect(() => {
    const refresh = () => setStyle(readSubtitleStyle(profileId));
    refresh();
    window.addEventListener(CHANGED_EVENT, refresh);
    return () => window.removeEventListener(CHANGED_EVENT, refresh);
  }, [profileId]);
  const update = useCallback((next: SubtitleStyle) => saveSubtitleStyle(profileId, next), [profileId]);
  return [style, update];
}

/** Lo mismo como estilo de un elemento cualquiera (la vista previa de Ajustes). */
export function subtitlePreviewStyle(style: SubtitleStyle): CSSProperties {
  return {
    fontSize: `${1.25 * (SIZE_SCALE[style.size] ?? 1)}rem`,
    color: COLOR_HEX[style.color],
    fontFamily: FONT_FAMILY[style.font],
    backgroundColor: style.look === "box" ? "rgb(0 0 0 / 0.7)" : "transparent",
    textShadow: style.look === "outline" ? OUTLINE_SHADOW : style.look === "shadow" ? "2px 2px 4px rgb(0 0 0 / 0.9)" : "none",
    padding: style.look === "box" ? "0.1em 0.4em" : 0,
  };
}

/** Regla CSS para los subtítulos de un `<video>` (reproductor del PC). */
export function subtitleCueCss(style: SubtitleStyle, scope: string): string {
  const preview = subtitlePreviewStyle(style);
  return `${scope} video::cue {
    font-size: ${Math.round(100 * (SIZE_SCALE[style.size] ?? 1))}%;
    color: ${preview.color};
    font-family: ${preview.fontFamily};
    background-color: ${preview.backgroundColor};
    text-shadow: ${preview.textShadow};
  }`;
}
