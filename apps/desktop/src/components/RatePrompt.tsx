import { useEffect } from "react";
import { useRouterState } from "@tanstack/react-router";
import { imagePath, useApiBase } from "../lib/api-client";
import { useActiveProfile } from "../lib/activeProfile";
import { dismissRatePrompt, useRateCandidate } from "../lib/ratePrompt";
import { RATING_OPTIONS, useRatings } from "../lib/ratings";
import { CloseIcon } from "./icons";

/** Se va sola si no se contesta. */
const AUTO_DISMISS_MS = 25_000;

/** «¿Qué te ha parecido…?» al terminar una película o un episodio, como en Netflix: la
 * valoración es lo que más afina las recomendaciones. Sale al volver del reproductor
 * (nunca encima del vídeo), una vez por título, y no si ya está valorado. */
export function RatePrompt() {
  const candidate = useRateCandidate();
  const { profileId } = useActiveProfile();
  const { data: base } = useApiBase();
  const { ratingOf, setRating } = useRatings();
  const inPlayer = useRouterState({ select: (state) => state.location.pathname.startsWith("/player/") });
  const relevant = candidate && candidate.profileId === profileId && ratingOf(candidate.kind, candidate.id) === null;
  const visible = Boolean(relevant) && !inPlayer;

  useEffect(() => {
    if (candidate && !relevant) dismissRatePrompt();
  }, [candidate, relevant]);

  useEffect(() => {
    if (!visible) return;
    const timer = window.setTimeout(dismissRatePrompt, AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [visible]);

  if (!visible || !candidate) return null;
  const poster = imagePath(base, candidate.posterPath, "w342");

  return (
    <div
      role="dialog"
      aria-label={`¿Qué te ha parecido ${candidate.title}?`}
      // Centrada abajo: a la derecha está la ventana pequeña del reproductor.
      className="preview-in fixed inset-x-3 bottom-24 z-[45] mx-auto flex max-w-md items-center gap-3 rounded-2xl bg-neutral-900/95 p-3 shadow-2xl shadow-black/60 ring-1 ring-white/10 backdrop-blur desk:bottom-6 ios:glass"
    >
      {poster && <img src={poster} alt="" className="h-16 w-11 shrink-0 rounded-md object-cover" />}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-semibold text-white">¿Qué te ha parecido {candidate.title}?</p>
        <div className="mt-2 flex gap-2">
          {RATING_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setRating(candidate.kind, candidate.id, option.value);
                dismissRatePrompt();
              }}
              className="flex items-center gap-1 whitespace-nowrap rounded-full bg-white/10 px-3 py-1.5 text-xs font-medium text-white outline-none transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-white"
              aria-label={option.label}
              title={option.label}
            >
              <span aria-hidden="true">{option.icon}</span>
              <span className="hidden sm:inline">{option.label}</span>
            </button>
          ))}
        </div>
      </div>
      <button
        type="button"
        onClick={dismissRatePrompt}
        className="self-start rounded-full p-1 text-neutral-400 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white"
        aria-label="Ahora no"
      >
        <CloseIcon className="h-4 w-4" />
      </button>
    </div>
  );
}
