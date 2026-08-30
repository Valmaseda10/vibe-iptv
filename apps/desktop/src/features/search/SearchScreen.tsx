import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import type { SearchResults } from "../../lib/api-types";

/** Buscador instantáneo con rebote de 150 ms sobre `GET /api/search` (FTS5). */
export function SearchScreen() {
  const [input, setInput] = useState("");
  const [debounced, setDebounced] = useState("");
  const navigate = useNavigate();

  useEffect(() => {
    const id = setTimeout(() => setDebounced(input), 150);
    return () => clearTimeout(id);
  }, [input]);

  const { data, isFetching } = useQuery({
    queryKey: ["search", debounced],
    queryFn: () => apiGet<SearchResults>(`/api/search?q=${encodeURIComponent(debounced)}`),
    enabled: debounced.trim().length > 0,
  });

  const hasQuery = debounced.trim().length > 0;
  const totalHits = (data?.movies.length ?? 0) + (data?.series.length ?? 0) + (data?.channels.length ?? 0);

  return (
    <div className="flex flex-col gap-6 px-6 py-8 sm:px-10">
      <h1 className="text-3xl font-semibold">Buscar</h1>
      <input
        autoFocus
        type="search"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="Título de canal, película o serie…"
        className="w-full max-w-xl rounded-lg border border-neutral-700 bg-neutral-900 px-4 py-3 text-lg outline-none focus-visible:border-red-500"
      />

      {hasQuery && !isFetching && totalHits === 0 && <p className="text-neutral-500">Sin resultados para "{debounced}".</p>}

      {data && data.movies.length > 0 && (
        <ResultSection title="Películas" hits={data.movies} onOpen={(id) => navigate({ to: "/movies/$movieId", params: { movieId: String(id) } })} />
      )}
      {data && data.series.length > 0 && (
        <ResultSection title="Series" hits={data.series} onOpen={(id) => navigate({ to: "/series/$seriesId", params: { seriesId: String(id) } })} />
      )}
      {data && data.channels.length > 0 && <ResultSection title="Canales" hits={data.channels} onOpen={() => {}} />}
    </div>
  );
}

function ResultSection({ title, hits, onOpen }: { title: string; hits: SearchResults["movies"]; onOpen: (id: number) => void }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral-500">{title}</h2>
      <ul className="flex flex-col gap-1">
        {hits.map((hit) => (
          <li key={`${hit.item_kind}-${hit.item_id}`}>
            <button
              type="button"
              onClick={() => onOpen(hit.item_id)}
              className="w-full rounded-lg px-3 py-2 text-left outline-none hover:bg-neutral-900"
            >
              {hit.title}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
