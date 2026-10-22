import { useState } from "react";
import { RATING_OPTIONS, useRatings } from "../lib/ratings";
import { DETAIL_ACTION } from "./detailAction";
import { GlassSheet, SHEET_ROW } from "./GlassSheet";

/** «Valorar» de las fichas (como en Netflix): No es para mí · Me gusta · Me encanta. La
 * valoración es del perfil, viaja por la nube y alimenta las recomendaciones. */
export function RatingControl({ kind, id, title }: { kind: "movie" | "series"; id: number; title: string }) {
  const { ratingOf, setRating } = useRatings();
  const [open, setOpen] = useState(false);
  const current = ratingOf(kind, id);
  const shown = RATING_OPTIONS.find((o) => o.value === current);

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={shown ? `Valoración: ${shown.label}. Cambiar` : "Valorar"}
        aria-haspopup="dialog"
        className={DETAIL_ACTION}
      >
        <span className={`text-xl leading-none sm:text-base ${shown ? "" : "opacity-60 grayscale"}`}>{shown?.icon ?? "👍"}</span>
        {shown?.label ?? "Valorar"}
      </button>
      {open && (
        <GlassSheet title={`Valorar «${title}»`} onClose={() => setOpen(false)}>
          {RATING_OPTIONS.map((option, index) => (
            <button
              key={option.value}
              type="button"
              autoFocus={index === 0}
              aria-pressed={current === option.value}
              onClick={() => {
                setRating(kind, id, option.value);
                setOpen(false);
              }}
              className={`${SHEET_ROW} ${current === option.value ? "bg-white/10" : ""}`}
            >
              <span className="w-10 text-center text-xl">{option.icon}</span>
              {option.label}
              {current === option.value && <span className="ml-auto text-sm text-white/60">✓</span>}
            </button>
          ))}
          {current !== null && (
            <button
              type="button"
              onClick={() => {
                setRating(kind, id, null);
                setOpen(false);
              }}
              className={SHEET_ROW}
            >
              <span className="w-10 text-center">✕</span>
              Quitar valoración
            </button>
          )}
          <button type="button" onClick={() => setOpen(false)} className={`${SHEET_ROW} mt-1 justify-center bg-white/10 font-semibold`}>
            Cancelar
          </button>
        </GlassSheet>
      )}
    </>
  );
}
