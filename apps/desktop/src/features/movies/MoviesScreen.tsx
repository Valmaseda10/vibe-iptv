import { useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { CatalogFilterBar, EMPTY_FILTERS, type CatalogFilters, type SortValue } from "../../components/CatalogFilterBar";
import type { MoviesPage } from "../../lib/api-types";
import { VirtualPosterGrid } from "../../components/VirtualPosterGrid";

type PageParam = { after?: number; afterKey?: string };

function sortParam(sort: SortValue): string | null {
  return sort === "recent" ? null : sort;
}

export function MoviesScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const [filters, setFilters] = useState<CatalogFilters>(EMPTY_FILTERS);

  const query = useInfiniteQuery({
    queryKey: ["movies", profileId, filters],
    queryFn: ({ pageParam }: { pageParam: PageParam }) => {
      const params = new URLSearchParams({ profile_id: String(profileId) });
      if (filters.genre != null) params.set("genre", String(filters.genre));
      if (filters.year != null) params.set("year", String(filters.year));
      if (filters.quality) params.set("quality", filters.quality);
      const sort = sortParam(filters.sort);
      if (sort) params.set("sort", sort);
      if (pageParam.after != null) params.set("after", String(pageParam.after));
      if (pageParam.afterKey != null) params.set("after_key", pageParam.afterKey);
      return apiGet<MoviesPage>(`/api/movies?${params.toString()}`);
    },
    initialPageParam: {} as PageParam,
    getNextPageParam: (lastPage) => (lastPage.next_cursor != null ? { after: lastPage.next_cursor, afterKey: lastPage.next_cursor_key ?? undefined } : undefined),
    enabled: profileId !== null,
  });

  const items = (query.data?.pages ?? []).flatMap((page) =>
    page.items.map((movie) => ({
      id: movie.id,
      title: movie.clean_title,
      posterPath: movie.poster_path,
      subtitle: movie.year ? String(movie.year) : undefined,
      isNew: movie.is_new,
    })),
  );

  return (
    <div className="flex h-full flex-col gap-6 py-8">
      <h1 className="px-6 text-3xl font-semibold sm:px-10">Películas</h1>
      <CatalogFilterBar kind="movie" filters={filters} onChange={setFilters} showQuality />
      <div className="min-h-0 flex-1">
        <VirtualPosterGrid
          items={items}
          hasMore={query.hasNextPage ?? false}
          isLoadingMore={query.isFetchingNextPage}
          isInitialLoading={query.isPending}
          onLoadMore={() => query.fetchNextPage()}
          onOpen={(id) => navigate({ to: "/movies/$movieId", params: { movieId: String(id) } })}
          emptyMessage="Nada con estos filtros. Prueba a quitar alguno, o añade un proveedor en Ajustes si el catálogo está vacío."
          favoriteKind="movie"
        />
      </div>
    </div>
  );
}
