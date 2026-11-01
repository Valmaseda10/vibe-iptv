import { useNavigate } from "@tanstack/react-router";
import { reviewYear, yearReviewSeason } from "../../lib/yearReview";

/** Tarjeta de «Tu año en VIBE» en Inicio, del 1 de diciembre al 15 de enero (F17). */
export function YearReviewBanner() {
  const navigate = useNavigate();
  if (!yearReviewSeason()) return null;
  const year = reviewYear();
  return (
    <div className="px-4 sm:px-10">
      <button
        type="button"
        onClick={() => navigate({ to: "/year" })}
        className="group relative flex w-full items-center gap-4 overflow-hidden rounded-2xl bg-[radial-gradient(circle_at_15%_20%,#e50914_0%,#5b0a12_50%,#141414_100%)] p-5 text-left outline-none ring-1 ring-white/10 transition-transform hover:scale-[1.01] focus-visible:ring-2 focus-visible:ring-white sm:p-6"
      >
        <span className="text-5xl font-black tracking-tight text-white/90 sm:text-6xl">{year}</span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-lg font-bold text-white sm:text-xl">Tu año en VIBE ya está aquí</span>
          <span className="text-sm text-white/75">Tus series, tus películas, tus horas y tu género del año.</span>
        </span>
        <span className="text-2xl text-white/80 transition-transform group-hover:translate-x-1">›</span>
      </button>
    </div>
  );
}
