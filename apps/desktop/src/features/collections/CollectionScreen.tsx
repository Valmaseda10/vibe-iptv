import { useEffect } from "react";
import { useCanGoBack, useNavigate, useParams, useRouter } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { apiGet, imagePath, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { CollectionResponse, SagaEntryResponse } from "../../lib/api-types";
import { FloatingBackButton } from "../../components/FloatingBackButton";
import { PRIMARY_ACTION } from "../../components/detailAction";
import { SkeletonBlock } from "../../components/Skeleton";

/** Una saga entera (F17): sus películas en orden de estreno, lo visto marcado, las que no
 * tiene el catálogo en gris, y un botón para seguir con la que toca. */
export function CollectionScreen() {
  const { collectionId } = useParams({ from: "/collections/$collectionId" });
  const navigate = useNavigate();
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const goBack = () => (canGoBack ? router.history.back() : navigate({ to: "/movies" }));
  const { data: base } = useApiBase();
  const { profileId } = useActiveProfile();
  const { data, isPending, isError } = useQuery({
    queryKey: ["collection", collectionId, profileId],
    queryFn: () => apiGet<CollectionResponse>(`/api/collections/${collectionId}?profile_id=${profileId}`),
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

  if (isPending) return <CollectionSkeleton />;
  if (isError || !data) return <p className="p-10 text-neutral-400">No se encontró esta saga.</p>;

  const backdrop = imagePath(base, data.backdrop_path, "w1280");
  const next = data.entries.find((entry) => entry.movie_id === data.next_movie_id);
  const started = data.watched > 0 || data.entries.some((entry) => entry.progress != null);
  const done = data.available > 0 && data.watched >= data.available;
  const percent = data.available > 0 ? Math.round((data.watched / data.available) * 100) : 0;

  return (
    <div className="relative min-h-full">
      <div className="absolute inset-0 h-[55vh] overflow-hidden">
        {backdrop && <img src={backdrop} alt="" className="h-full w-full object-cover" />}
        <div className="absolute inset-0 bg-gradient-to-t from-neutral-950 via-neutral-950/70 to-transparent ios:from-black ios:via-black/60" />
        <div className="absolute inset-0 bg-gradient-to-r from-neutral-950/90 via-neutral-950/10 to-transparent ios:from-black/70 ios:via-black/10" />
      </div>

      <div className={`relative flex flex-col gap-5 px-4 pb-16 sm:px-10 ${backdrop ? "pt-[28vh]" : "pt-10"}`}>
        <FloatingBackButton onClick={goBack} />
        <button type="button" onClick={goBack} className="w-fit text-sm text-neutral-400 outline-none hover:text-neutral-200 ios:hidden ios:desk:block">
          ← Volver <span className="pointer-coarse:hidden">(Esc)</span>
        </button>
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-red-500">Saga</p>
        <h1 className="-mt-3 max-w-3xl text-4xl font-bold sm:text-5xl">{data.name}</h1>
        <div className="flex max-w-md flex-col gap-2">
          <p className="text-sm text-neutral-300">
            {data.entries.length} películas
            {data.available < data.entries.length ? ` · ${data.available} en tu lista` : ""}
            {started ? ` · Vistas ${data.watched} de ${data.available}` : ""}
          </p>
          {started && (
            <div className="h-1 overflow-hidden rounded bg-neutral-700" aria-hidden>
              <div className="h-full bg-red-600 transition-[width] duration-500" style={{ width: `${percent}%` }} />
            </div>
          )}
        </div>
        {data.overview && <p className="max-w-2xl text-neutral-300">{data.overview}</p>}

        {next && next.movie_id != null && (
          <div className="pt-1">
            <button
              type="button"
              autoFocus
              onClick={() => navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(next.movie_id) } })}
              className={PRIMARY_ACTION}
            >
              ▶ {started ? `Seguir con ${next.title}` : `Empezar por ${next.title}`}
            </button>
          </div>
        )}
        {done && <p className="text-sm font-medium text-emerald-400">✓ Has visto toda la saga</p>}

        <ol className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {data.entries.map((entry, index) => (
            <SagaEntryCard
              key={`${entry.movie_id ?? "x"}-${index}`}
              entry={entry}
              index={index}
              isNext={entry.movie_id != null && entry.movie_id === data.next_movie_id}
              onOpen={entry.movie_id != null ? () => navigate({ to: "/movies/$movieId", params: { movieId: String(entry.movie_id) } }) : undefined}
            />
          ))}
        </ol>
      </div>
    </div>
  );
}

function SagaEntryCard({ entry, index, isNext, onOpen }: { entry: SagaEntryResponse; index: number; isNext: boolean; onOpen?: () => void }) {
  const { data: base } = useApiBase();
  const poster = imagePath(base, entry.poster_path, "w185");
  const missing = entry.movie_id == null;
  const content = (
    <>
      <span className="relative h-24 w-16 shrink-0 overflow-hidden rounded-md bg-neutral-800 ring-1 ring-white/10">
        {poster && <img src={poster} alt="" loading="lazy" className={`h-full w-full object-cover ${missing ? "opacity-40 grayscale" : ""}`} />}
        {entry.progress != null && (
          <span className="absolute inset-x-0 bottom-0 h-1 bg-neutral-700">
            <span className="block h-full bg-red-600" style={{ width: `${Math.round(entry.progress * 100)}%` }} />
          </span>
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1 text-left">
        <span className="text-xs font-semibold tabular-nums text-neutral-500">{index + 1}</span>
        <span className={`line-clamp-2 font-medium leading-tight ${missing ? "text-neutral-500" : "text-neutral-100"}`}>{entry.title}</span>
        <span className="text-xs text-neutral-500">
          {entry.year ?? ""}
          {missing ? `${entry.year ? " · " : ""}No está en tu lista` : ""}
        </span>
      </span>
      {entry.watched && (
        <span className="self-center text-lg text-emerald-400" aria-label="Vista">
          ✓
        </span>
      )}
      {isNext && !entry.watched && <span className="self-center rounded-full bg-red-600 px-2 py-0.5 text-[11px] font-semibold text-white">Siguiente</span>}
    </>
  );
  const frame = `flex w-full items-center gap-3 rounded-xl p-2 ${isNext ? "bg-white/[0.07] ring-1 ring-red-600/50" : "bg-white/[0.03]"}`;
  return (
    <li>
      {onOpen ? (
        <button type="button" onClick={onOpen} className={`${frame} outline-none transition-colors hover:bg-white/10 focus-visible:ring-2 focus-visible:ring-white/70`}>
          {content}
        </button>
      ) : (
        <div className={frame} aria-disabled>
          {content}
        </div>
      )}
    </li>
  );
}

function CollectionSkeleton() {
  return (
    <div className="flex flex-col gap-4 px-4 pt-24 sm:px-10">
      <SkeletonBlock className="h-4 w-16" />
      <SkeletonBlock className="h-12 w-80 max-w-full" />
      <SkeletonBlock className="h-4 w-56" />
      <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        {Array.from({ length: 6 }, (_, i) => (
          <SkeletonBlock key={i} className="h-28 rounded-xl" />
        ))}
      </div>
    </div>
  );
}
