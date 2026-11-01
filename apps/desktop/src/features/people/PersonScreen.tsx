import { useEffect, useState } from "react";
import { useCanGoBack, useNavigate, useParams, useRouter } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, imagePath, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { PersonResponse } from "../../lib/api-types";
import { FloatingBackButton } from "../../components/FloatingBackButton";
import { PosterCard } from "../../components/PosterCard";

const DEPARTMENTS: Record<string, string> = {
  Acting: "Interpretación",
  Directing: "Dirección",
  Writing: "Guion",
  Production: "Producción",
};

/** «12 de marzo de 1964» a partir de «1964-03-12». */
function longDate(iso: string | null): string | null {
  if (!iso) return null;
  const date = new Date(`${iso}T12:00:00`);
  return Number.isNaN(date.getTime()) ? null : date.toLocaleDateString("es-ES", { day: "numeric", month: "long", year: "numeric" });
}

function ageAt(birthday: string | null, deathday: string | null): number | null {
  if (!birthday) return null;
  const birth = new Date(`${birthday}T12:00:00`);
  const end = deathday ? new Date(`${deathday}T12:00:00`) : new Date();
  if (Number.isNaN(birth.getTime())) return null;
  let age = end.getFullYear() - birth.getFullYear();
  if (end.getMonth() < birth.getMonth() || (end.getMonth() === birth.getMonth() && end.getDate() < birth.getDate())) age--;
  return age;
}

/** Ficha de una persona (F17): foto, biografía y todo lo suyo que hay en el catálogo. */
export function PersonScreen() {
  const { personId } = useParams({ from: "/people/$personId" });
  const navigate = useNavigate();
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const goBack = () => (canGoBack ? router.history.back() : navigate({ to: "/" }));
  const { data: base } = useApiBase();
  const { profileId } = useActiveProfile();
  const [bioOpen, setBioOpen] = useState(false);
  const { data, isPending, isError } = useQuery({
    queryKey: ["person", personId, profileId],
    queryFn: () => apiGet<PersonResponse>(`/api/people/${personId}?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") goBack();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [canGoBack]);

  if (isPending) return <PersonSkeleton />;
  if (isError || !data) return <p className="p-10 text-neutral-400">No hay nada de esta persona en tu lista.</p>;

  const photo = imagePath(base, data.profile_path, "w500");
  const age = ageAt(data.birthday, data.deathday);
  const born = longDate(data.birthday);
  const died = longDate(data.deathday);
  const department = data.department ? (DEPARTMENTS[data.department] ?? null) : null;
  const longBio = (data.biography?.length ?? 0) > 420;

  return (
    <div className="relative flex min-h-full flex-col gap-8 px-4 pb-16 pt-10 sm:px-10">
      <FloatingBackButton onClick={goBack} />
      <button type="button" onClick={goBack} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200 ios:hidden ios:desk:block">
        ← Volver <span className="pointer-coarse:hidden">(Esc)</span>
      </button>

      <div className="flex flex-col items-center gap-6 sm:flex-row sm:items-start">
        <div className="h-48 w-48 shrink-0 overflow-hidden rounded-full bg-neutral-800 shadow-2xl ring-1 ring-white/10 sm:h-56 sm:w-56">
          {photo && <img src={photo} alt="" className="h-full w-full object-cover" />}
        </div>
        <div className="flex max-w-3xl flex-col gap-3 text-center sm:text-left">
          {department && <p className="text-xs font-semibold uppercase tracking-[0.2em] text-red-500">{department}</p>}
          <h1 className="text-4xl font-bold sm:text-5xl">{data.name}</h1>
          <p className="text-sm text-neutral-400">
            {born && `${died ? "" : "Nació el "}${born}${data.place_of_birth ? ` en ${data.place_of_birth}` : ""}`}
            {died && ` · Murió el ${died}`}
            {age != null && ` (${age} años)`}
          </p>
          {data.biography && (
            <div className="flex flex-col gap-1">
              <p className={`whitespace-pre-line text-neutral-300 ${longBio && !bioOpen ? "line-clamp-5" : ""}`}>{data.biography}</p>
              {longBio && (
                <button type="button" onClick={() => setBioOpen((open) => !open)} className="w-fit text-sm font-medium text-neutral-400 outline-none hover:text-white focus-visible:text-white">
                  {bioOpen ? "Ver menos" : "Leer más"}
                </button>
              )}
            </div>
          )}
        </div>
      </div>

      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-semibold text-neutral-200">En tu lista · {data.titles.length}</h2>
        <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8">
          {data.titles.map((title) => (
            <PosterCard
              key={`${title.kind}-${title.id}`}
              fluid
              title={title.clean_title}
              posterPath={title.poster_path}
              subtitle={[title.year, title.role].filter(Boolean).join(" · ") || undefined}
              menuItem={title.kind === "movie" || title.kind === "series" ? { kind: title.kind, id: title.id } : undefined}
              onOpen={() =>
                title.kind === "series"
                  ? navigate({ to: "/series/$seriesId", params: { seriesId: String(title.id) } })
                  : navigate({ to: "/movies/$movieId", params: { movieId: String(title.id) } })
              }
            />
          ))}
        </div>
      </section>
    </div>
  );
}

function PersonSkeleton() {
  return (
    <div className="flex flex-col gap-8 px-4 pt-20 sm:px-10">
      <div className="flex flex-col items-center gap-6 sm:flex-row">
        <div className="h-48 w-48 animate-pulse rounded-full bg-neutral-800" />
        <div className="flex w-full max-w-xl flex-col gap-3">
          <div className="h-10 w-72 max-w-full animate-pulse rounded bg-neutral-800" />
          <div className="h-4 w-60 animate-pulse rounded bg-neutral-800" />
          <div className="h-20 w-full animate-pulse rounded bg-neutral-900" />
        </div>
      </div>
    </div>
  );
}
