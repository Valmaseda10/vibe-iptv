import { useCallback, useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, imagePath, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { ProfileSummary, YearReviewResponse, YearTitleResponse } from "../../lib/api-types";
import { reviewYear } from "../../lib/yearReview";

/** Lo que dura cada pantalla antes de pasar sola. */
const SLIDE_MS = 8_000;
const WEEKDAYS = ["los lunes", "los martes", "los miércoles", "los jueves", "los viernes", "los sábados", "los domingos"];

function hours(seconds: number): string {
  const h = seconds / 3600;
  return h >= 10 ? Math.round(h).toLocaleString("es-ES") : h.toLocaleString("es-ES", { maximumFractionDigits: 1 });
}

function moment(hour: number): string {
  if (hour < 6) return "de madrugada";
  if (hour < 13) return "por la mañana";
  if (hour < 16) return "a la hora de comer";
  if (hour < 21) return "por la tarde";
  return "por la noche";
}

/** «Tu año en VIBE» (F17): el resumen del año del perfil, pantalla a pantalla como una
 * historia (flechas, toques a los lados o pasa sola). Suma lo visto en todos sus aparatos. */
export function YearReviewScreen() {
  const navigate = useNavigate();
  const year = reviewYear();
  const { profileId } = useActiveProfile();
  const profiles = useQuery({ queryKey: ["profiles"], queryFn: () => apiGet<ProfileSummary[]>("/api/profiles") });
  const profile = profiles.data?.find((p) => p.id === profileId);
  const { data, isPending } = useQuery({
    queryKey: ["year-review", profileId, year],
    queryFn: () => apiGet<YearReviewResponse>(`/api/year-review?profile_id=${profileId}&year=${year}`),
    enabled: profileId !== null,
  });
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  const close = useCallback(() => navigate({ to: "/" }), [navigate]);

  const slides = data ? buildSlides(data, profile?.name ?? "") : [];
  const count = slides.length;
  const go = useCallback((delta: number) => setIndex((i) => Math.max(0, Math.min(count - 1, i + delta))), [count]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Backspace") close();
      else if (e.key === "ArrowRight" || e.key === " " || e.key === "Enter") go(1);
      else if (e.key === "ArrowLeft") go(-1);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [go, close]);

  useEffect(() => {
    if (paused || count === 0 || index >= count - 1) return;
    const timer = window.setTimeout(() => go(1), SLIDE_MS);
    return () => window.clearTimeout(timer);
  }, [index, paused, count, go]);

  const slide = slides[Math.min(index, Math.max(0, count - 1))];
  // En `body`: a pantalla completa por encima del menú y de cualquier `transform` de la app.
  return createPortal(
    <div className="fixed inset-0 z-[60] flex flex-col overflow-hidden bg-black text-white" onPointerDown={() => setPaused(true)} onPointerUp={() => setPaused(false)}>
      <div className={`absolute inset-0 transition-[background] duration-700 ${slide?.background ?? "bg-neutral-950"}`} />
      <div className="relative flex gap-1.5 px-4 pt-[max(1rem,env(safe-area-inset-top))]">
        {slides.map((s, i) => (
          <div key={s.key} className="h-1 flex-1 overflow-hidden rounded-full bg-white/25">
            <div
              className="h-full bg-white"
              style={{ width: i < index ? "100%" : i > index ? "0%" : "100%", transition: i === index && !paused ? `width ${SLIDE_MS}ms linear` : "none" }}
              key={i === index ? `on-${index}` : `off-${i}`}
            />
          </div>
        ))}
      </div>
      <button
        type="button"
        onClick={close}
        aria-label="Cerrar"
        className="absolute right-4 top-[max(1.75rem,calc(env(safe-area-inset-top)+0.75rem))] z-10 flex h-10 w-10 items-center justify-center rounded-full bg-black/30 text-xl outline-none hover:bg-black/50 focus-visible:ring-2 focus-visible:ring-white"
      >
        ✕
      </button>

      <div className="relative flex flex-1 items-center justify-center px-6 py-10">
        {isPending && <p className="animate-pulse text-white/70">Preparando tu año…</p>}
        {slide && (
          <div key={slide.key} className="year-slide flex w-full max-w-3xl flex-col items-center gap-6 text-center">
            {slide.content}
          </div>
        )}
      </div>

      {/* Toques a los lados para pasar, como en una historia. */}
      <button type="button" aria-label="Anterior" className="absolute bottom-0 left-0 top-20 w-1/4 outline-none" onClick={() => go(-1)} />
      <button type="button" aria-label="Siguiente" className="absolute bottom-0 right-0 top-20 w-1/4 outline-none" onClick={() => go(1)} />
      <p className="relative pb-[max(1rem,env(safe-area-inset-bottom))] text-center text-xs text-white/50 pointer-coarse:hidden">← → para moverte · Esc para salir</p>
    </div>,
    document.body,
  );
}

type Slide = { key: string; background: string; content: ReactNode };

function Big({ children }: { children: ReactNode }) {
  return <p className="text-6xl font-black leading-none tracking-tight sm:text-8xl">{children}</p>;
}

function Kicker({ children }: { children: ReactNode }) {
  return <p className="text-sm font-semibold uppercase tracking-[0.25em] text-white/70">{children}</p>;
}

function Poster({ title, size = "md" }: { title: YearTitleResponse; size?: "md" | "lg" }) {
  const { data: base } = useApiBase();
  const src = imagePath(base, title.poster_path, size === "lg" ? "w500" : "w342");
  return (
    <div className={`overflow-hidden rounded-xl bg-white/10 shadow-2xl ring-1 ring-white/20 ${size === "lg" ? "w-40 sm:w-52" : "w-24 sm:w-28"} aspect-[2/3]`}>
      {src ? <img src={src} alt="" className="h-full w-full object-cover" /> : <div className="flex h-full items-center justify-center p-2 text-xs">{title.title}</div>}
    </div>
  );
}

function buildSlides(data: YearReviewResponse, name: string): Slide[] {
  const slides: Slide[] = [];
  const now = new Date();
  const partial = data.year === now.getFullYear() && now.getMonth() < 11;
  slides.push({
    key: "intro",
    background: "bg-[radial-gradient(circle_at_30%_20%,#e50914_0%,#5b0a12_45%,#0a0a0a_100%)]",
    content: (
      <>
        <Kicker>{partial ? "Lo que llevas de año" : "Tu año en VIBE"}</Kicker>
        <Big>{data.year}</Big>
        <p className="text-2xl font-semibold">{name ? `${name}, esto es lo que has visto` : "Esto es lo que has visto"}</p>
      </>
    ),
  });
  if (data.total_seconds < 600) {
    slides.push({
      key: "empty",
      background: "bg-neutral-900",
      content: <p className="text-2xl font-semibold">Aún casi no has visto nada este año. Vuelve cuando hayas visto unas cuantas cosas.</p>,
    });
    return slides;
  }
  const parts = [
    { label: "Series", seconds: data.series_seconds, color: "bg-sky-400" },
    { label: "Películas", seconds: data.movie_seconds, color: "bg-amber-400" },
    { label: "Directo", seconds: data.live_seconds, color: "bg-emerald-400" },
  ].filter((p) => p.seconds > 0);
  slides.push({
    key: "hours",
    background: "bg-[linear-gradient(160deg,#1e1b4b_0%,#4c1d95_55%,#0a0a0a_100%)]",
    content: (
      <>
        <Kicker>Has pasado</Kicker>
        <Big>{hours(data.total_seconds)} h</Big>
        <p className="text-xl">{data.days_watched === 1 ? "viendo cosas en un solo día" : `viendo cosas en ${data.days_watched} días distintos`}</p>
        <div className="flex w-full max-w-md flex-col gap-2">
          <div className="flex h-3 w-full overflow-hidden rounded-full bg-white/10">
            {parts.map((p) => (
              <div key={p.label} className={p.color} style={{ width: `${(p.seconds / data.total_seconds) * 100}%` }} />
            ))}
          </div>
          <div className="flex justify-center gap-4 text-sm text-white/80">
            {parts.map((p) => (
              <span key={p.label} className="flex items-center gap-1.5">
                <span className={`h-2 w-2 rounded-full ${p.color}`} />
                {p.label} · {hours(p.seconds)} h
              </span>
            ))}
          </div>
        </div>
        {data.estimated_seconds > 0 && <p className="text-xs text-white/50">Lo de antes de noviembre de 2026 es una estimación: entonces la app aún no lo apuntaba.</p>}
      </>
    ),
  });
  const [topSeries, ...moreSeries] = data.top_series;
  if (topSeries) {
    slides.push({
      key: "series",
      background: "bg-[linear-gradient(200deg,#0c4a6e_0%,#082f49_50%,#0a0a0a_100%)]",
      content: (
        <>
          <Kicker>Tu serie del año</Kicker>
          <Poster title={topSeries} size="lg" />
          <p className="text-3xl font-bold sm:text-4xl">{topSeries.title}</p>
          <p className="text-white/80">{hours(topSeries.seconds)} horas · {data.episodes_watched} episodios en total</p>
          {moreSeries.length > 0 && (
            <ol className="flex flex-col gap-1 text-sm text-white/70">
              {moreSeries.map((s, i) => (
                <li key={s.title}>
                  {i + 2}. {s.title}
                </li>
              ))}
            </ol>
          )}
        </>
      ),
    });
  }
  if (data.top_movies.length > 0) {
    slides.push({
      key: "movies",
      background: "bg-[linear-gradient(160deg,#78350f_0%,#451a03_55%,#0a0a0a_100%)]",
      content: (
        <>
          <Kicker>Películas</Kicker>
          <Big>{data.movies_watched}</Big>
          <p className="text-xl">{data.movies_watched === 1 ? "película este año:" : "películas este año. Las que más:"}</p>
          <div className="flex flex-wrap justify-center gap-3">
            {data.top_movies.slice(0, 5).map((m) => (
              <Poster key={m.title} title={m} />
            ))}
          </div>
        </>
      ),
    });
  }
  if (data.top_genres.length > 0) {
    slides.push({
      key: "genres",
      background: "bg-[linear-gradient(200deg,#831843_0%,#500724_55%,#0a0a0a_100%)]",
      content: (
        <>
          <Kicker>Lo tuyo es</Kicker>
          <Big>{data.top_genres[0]}</Big>
          {data.top_genres.length > 1 && (
            <div className="flex flex-wrap justify-center gap-2">
              {data.top_genres.slice(1).map((g) => (
                <span key={g} className="rounded-full bg-white/15 px-4 py-1.5 text-sm">
                  {g}
                </span>
              ))}
            </div>
          )}
        </>
      ),
    });
  }
  if (data.favorite_hour != null) {
    slides.push({
      key: "moment",
      background: "bg-[linear-gradient(180deg,#0f172a_0%,#1e3a8a_60%,#0a0a0a_100%)]",
      content: (
        <>
          <Kicker>Tu momento</Kicker>
          <Big>{String(data.favorite_hour).padStart(2, "0")}:00</Big>
          <p className="text-xl">
            Ves más {moment(data.favorite_hour)}
            {data.favorite_weekday != null ? `, sobre todo ${WEEKDAYS[data.favorite_weekday]}` : ""}.
          </p>
          {data.longest_streak > 1 && (
            <p className="text-lg text-white/80">
              Tu mejor racha: <strong>{data.longest_streak} días seguidos</strong>
            </p>
          )}
          {data.busiest_day && data.busiest_day_seconds != null && (
            <p className="text-sm text-white/60">
              Tu día más intenso fue el{" "}
              {new Date(`${data.busiest_day}T12:00:00`).toLocaleDateString("es-ES", { day: "numeric", month: "long" })}: {hours(data.busiest_day_seconds)} horas
            </p>
          )}
        </>
      ),
    });
  }
  if (data.live_seconds > 0 && data.top_channels[0]) {
    slides.push({
      key: "live",
      background: "bg-[linear-gradient(160deg,#064e3b_0%,#022c22_55%,#0a0a0a_100%)]",
      content: (
        <>
          <Kicker>En directo</Kicker>
          <Big>{hours(data.live_seconds)} h</Big>
          <p className="text-xl">
            de televisión en directo. Tu canal: <strong>{data.top_channels[0].title}</strong>
          </p>
        </>
      ),
    });
  }
  slides.push({
    key: "summary",
    background: "bg-[radial-gradient(circle_at_70%_10%,#e50914_0%,#3b0a0e_45%,#0a0a0a_100%)]",
    content: (
      <>
        <Kicker>
          {name ? `${name} · ` : ""}
          {data.year} en VIBE
        </Kicker>
        <div className="grid w-full max-w-lg grid-cols-2 gap-3 text-left">
          <Stat label="Horas" value={hours(data.total_seconds)} />
          <Stat label="Días" value={String(data.days_watched)} />
          <Stat label="Películas" value={String(data.movies_watched)} />
          <Stat label="Episodios" value={String(data.episodes_watched)} />
          {data.top_series[0] && <Stat label="Serie" value={data.top_series[0].title} wide />}
          {data.top_genres[0] && <Stat label="Género" value={data.top_genres[0]} wide />}
        </div>
        <p className="text-sm text-white/60">Gracias por verlo con VIBE.</p>
      </>
    ),
  });
  return slides;
}

function Stat({ label, value, wide }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={`rounded-2xl bg-white/10 p-4 ${wide ? "col-span-2" : ""}`}>
      <p className="text-xs uppercase tracking-widest text-white/60">{label}</p>
      <p className="truncate text-2xl font-bold">{value}</p>
    </div>
  );
}
