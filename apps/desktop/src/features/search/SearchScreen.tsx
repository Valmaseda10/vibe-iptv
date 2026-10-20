import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useActiveProfile } from "../../lib/activeProfile";
import { apiGet, imagePath, useApiBase } from "../../lib/api-client";
import type { SearchResults } from "../../lib/api-types";

/** Buscador instantáneo con rebote de 150 ms sobre `GET /api/search` (FTS5). Portadas en
 * vez de una lista de texto plano, y flechas para moverse entre resultados — mismo
 * criterio de accesibilidad por mando que el resto de la app (F "premium"). */
export function SearchScreen() {
  const [input, setInput] = useState("");
  const [debounced, setDebounced] = useState("");
  const navigate = useNavigate();
  const resultsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const id = setTimeout(() => setDebounced(input), 150);
    return () => clearTimeout(id);
  }, [input]);

  const { profileId } = useActiveProfile();
  // Lo que el perfil no puede ver no sale en la búsqueda (F14): el núcleo filtra por perfil.
  const { data, isFetching } = useQuery({
    queryKey: ["search", debounced, profileId],
    queryFn: () => apiGet<SearchResults>(`/api/search?q=${encodeURIComponent(debounced)}&profile_id=${profileId}`),
    enabled: debounced.trim().length > 0 && profileId !== null,
  });

  const hasQuery = debounced.trim().length > 0;
  const totalHits = (data?.movies.length ?? 0) + (data?.series.length ?? 0) + (data?.channels.length ?? 0);

  // Flecha abajo desde el campo de texto entra en la primera tarjeta de resultados;
  // dentro de los resultados, las flechas se mueven por los <button> tal cual salen en
  // el DOM (una única rejilla lógica, sin distinguir de qué sección es cada uno).
  function onInputKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key !== "ArrowDown") return;
    const first = resultsRef.current?.querySelector<HTMLElement>("button");
    if (first) {
      e.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="flex flex-col gap-5 px-4 py-5 sm:gap-6 sm:px-10 sm:py-8">
      <h1 className="text-2xl font-semibold sm:text-3xl">Buscar</h1>
      <input
        autoFocus
        type="search"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={onInputKeyDown}
        placeholder="Título de canal, película o serie…"
        className="w-full max-w-xl rounded-lg border border-neutral-700 bg-neutral-900 px-4 py-3 text-lg outline-none focus-visible:border-red-500"
      />

      {hasQuery && !isFetching && totalHits === 0 && <p className="text-neutral-500">Sin resultados para "{debounced}".</p>}

      <div ref={resultsRef} className="flex flex-col gap-8">
        {data && data.movies.length > 0 && (
          <ResultSection title="Películas" hits={data.movies} onOpen={(id) => navigate({ to: "/movies/$movieId", params: { movieId: String(id) } })} />
        )}
        {data && data.series.length > 0 && (
          <ResultSection title="Series" hits={data.series} onOpen={(id) => navigate({ to: "/series/$seriesId", params: { seriesId: String(id) } })} />
        )}
        {data && data.channels.length > 0 && (
          <ResultSection title="Canales" hits={data.channels} onOpen={(id) => navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(id) } })} />
        )}
      </div>
    </div>
  );
}

function ResultSection({ title, hits, onOpen }: { title: string; hits: SearchResults["movies"]; onOpen: (id: number) => void }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral-500">{title}</h2>
      <div className="flex flex-wrap gap-4">
        {hits.map((hit) => (
          <SearchResultCard key={`${hit.item_kind}-${hit.item_id}`} title={hit.title} posterPath={hit.poster_path} onOpen={() => onOpen(hit.item_id)} />
        ))}
      </div>
    </section>
  );
}

function SearchResultCard({ title, posterPath, onOpen }: { title: string; posterPath: string | null; onOpen: () => void }) {
  const { data: base } = useApiBase();
  const src = imagePath(base, posterPath, "w342");

  return (
    <button
      type="button"
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      className="group flex w-32 shrink-0 flex-col gap-2 rounded text-left outline-none sm:w-36"
    >
      <div className="aspect-[2/3] w-full overflow-hidden rounded-lg bg-neutral-800 ring-1 ring-white/5 transition-transform group-hover:scale-[1.03] group-focus-visible:scale-[1.03]">
        {src ? (
          <img src={src} alt="" loading="lazy" className="h-full w-full object-cover" />
        ) : (
          <div className="flex h-full w-full items-center justify-center p-2 text-center text-xs text-neutral-500">{title}</div>
        )}
      </div>
      <p className="truncate text-sm text-neutral-100">{title}</p>
    </button>
  );
}
