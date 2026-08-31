import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import type { MoviesPage } from "../../lib/api-types";
import { VirtualPosterGrid } from "../../components/VirtualPosterGrid";

export function MoviesScreen() {
  const navigate = useNavigate();
  const query = useInfiniteQuery({
    queryKey: ["movies"],
    queryFn: ({ pageParam }: { pageParam: number | undefined }) => apiGet<MoviesPage>(`/api/movies${pageParam ? `?after=${pageParam}` : ""}`),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
  });

  const items = (query.data?.pages ?? []).flatMap((page) =>
    page.items.map((movie) => ({
      id: movie.id,
      title: movie.clean_title,
      posterPath: movie.poster_path,
      subtitle: movie.year ? String(movie.year) : undefined,
    })),
  );

  return (
    <div className="flex h-full flex-col gap-6 py-8">
      <h1 className="px-6 text-3xl font-semibold sm:px-10">Películas</h1>
      <div className="min-h-0 flex-1">
        <VirtualPosterGrid
          items={items}
          hasMore={query.hasNextPage ?? false}
          isLoadingMore={query.isFetchingNextPage}
          onLoadMore={() => query.fetchNextPage()}
          onOpen={(id) => navigate({ to: "/movies/$movieId", params: { movieId: String(id) } })}
          emptyMessage="Todavía no hay películas sincronizadas. Añade un proveedor en Ajustes."
          favoriteKind="movie"
        />
      </div>
    </div>
  );
}
