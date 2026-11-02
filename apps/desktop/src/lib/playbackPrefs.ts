import { useCallback, useEffect, useState } from "react";
import { useActiveProfile } from "./activeProfile";

/** Ajustes → Reproducción (2026-11-02), por perfil y en cada aparato, como los subtítulos:
 * el siguiente episodio solo o no y con cuánta cuenta atrás, cuánto retroceder al reanudar
 * y el idioma de audio preferido. Los usan el reproductor del PC y el de iPhone/iPad. */
export type PlaybackPrefs = {
  autoplayNext: boolean;
  nextCountdown: number;
  resumeRewind: number;
  audioLanguage: AudioLanguage;
};

export type AudioLanguage = "auto" | "es" | "en";

export const NEXT_COUNTDOWN_OPTIONS = [
  { value: 5, label: "5 s" },
  { value: 10, label: "10 s" },
  { value: 15, label: "15 s" },
  { value: 20, label: "20 s" },
] as const;

export const RESUME_REWIND_OPTIONS = [
  { value: 0, label: "Nada" },
  { value: 5, label: "5 s" },
  { value: 10, label: "10 s" },
  { value: 30, label: "30 s" },
] as const;

export const AUDIO_LANGUAGE_OPTIONS: readonly { value: AudioLanguage; label: string }[] = [
  { value: "auto", label: "El último elegido" },
  { value: "es", label: "Español" },
  { value: "en", label: "Inglés" },
];

/** Cómo se llama cada idioma en las pistas (ffprobe da «spa»/«eng»; VLC, el nombre). */
const LANGUAGE_WORDS: Record<Exclude<AudioLanguage, "auto">, string[]> = {
  es: ["spa", "es", "esp", "cas", "spanish", "español", "espanol", "castellano", "castilian"],
  en: ["eng", "en", "english", "inglés", "ingles"],
};

export const DEFAULT_PLAYBACK_PREFS: PlaybackPrefs = { autoplayNext: true, nextCountdown: 10, resumeRewind: 5, audioLanguage: "auto" };

const CHANGED_EVENT = "vibe-playback-prefs";
const storageKey = (profileId: number | null) => `vibe.playback.${profileId ?? "sin-perfil"}`;

function valid(prefs: Partial<PlaybackPrefs> | null): PlaybackPrefs {
  const pickNumber = (value: unknown, options: readonly { value: number }[], fallback: number) =>
    typeof value === "number" && options.some((o) => o.value === value) ? value : fallback;
  return {
    autoplayNext: typeof prefs?.autoplayNext === "boolean" ? prefs.autoplayNext : DEFAULT_PLAYBACK_PREFS.autoplayNext,
    nextCountdown: pickNumber(prefs?.nextCountdown, NEXT_COUNTDOWN_OPTIONS, DEFAULT_PLAYBACK_PREFS.nextCountdown),
    resumeRewind: pickNumber(prefs?.resumeRewind, RESUME_REWIND_OPTIONS, DEFAULT_PLAYBACK_PREFS.resumeRewind),
    audioLanguage: AUDIO_LANGUAGE_OPTIONS.some((o) => o.value === prefs?.audioLanguage) ? (prefs?.audioLanguage as AudioLanguage) : "auto",
  };
}

export function readPlaybackPrefs(profileId: number | null): PlaybackPrefs {
  try {
    const raw = window.localStorage.getItem(storageKey(profileId));
    return valid(raw ? (JSON.parse(raw) as Partial<PlaybackPrefs>) : null);
  } catch {
    return DEFAULT_PLAYBACK_PREFS;
  }
}

export function savePlaybackPrefs(profileId: number | null, prefs: PlaybackPrefs) {
  try {
    window.localStorage.setItem(storageKey(profileId), JSON.stringify(valid(prefs)));
  } catch {
    // Sin almacenamiento: vale para esta sesión, no se recuerda.
  }
  window.dispatchEvent(new Event(CHANGED_EVENT));
}

/** Las preferencias del perfil activo; se ponen al día si cambian en otra pantalla. */
export function usePlaybackPrefs(): [PlaybackPrefs, (next: PlaybackPrefs) => void] {
  const { profileId } = useActiveProfile();
  const [prefs, setPrefs] = useState(() => readPlaybackPrefs(profileId));
  useEffect(() => {
    const refresh = () => setPrefs(readPlaybackPrefs(profileId));
    refresh();
    window.addEventListener(CHANGED_EVENT, refresh);
    return () => window.removeEventListener(CHANGED_EVENT, refresh);
  }, [profileId]);
  const update = useCallback((next: PlaybackPrefs) => savePlaybackPrefs(profileId, next), [profileId]);
  return [prefs, update];
}

/** ¿Esta pista (idioma de ffprobe o nombre de VLC) es del idioma preferido? */
export function trackMatchesLanguage(track: string | null | undefined, language: AudioLanguage): boolean {
  if (!track || language === "auto") return false;
  const words = track.toLowerCase().split(/[^a-zñáéíóú]+/).filter(Boolean);
  return LANGUAGE_WORDS[language].some((w) => words.includes(w));
}

/** Para el reproductor de iPhone/iPad: lo que hay que buscar en el nombre de la pista. */
export function nativeAudioPreference(language: AudioLanguage): string | null {
  return language === "auto" ? null : `lang:${LANGUAGE_WORDS[language].join(",")}`;
}
