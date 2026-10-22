import { useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { CatalogFilterBar, EMPTY_FILTERS, type CatalogFilters, type SortValue } from "../../components/CatalogFilterBar";
import type { SeriesPage } from "../../lib/api-types";
import { VirtualPosterGrid } from "../../components/VirtualPosterGrid";
import { recallScreenState, rememberScreenState } from "../../lib/screenMemory";
import { LargeTitle } from "../../components/LargeTitle";
import { BackIcon } from "../../components/icons";

type PageParam = { after?: number; afterKey?: string };

const FILTERS_MEMORY_KEY = "series:filters";

function sortParam(sort: SortValue): string | null {
  return sort === "recent" ? null : sort;
}

/** Todo el catálogo de series con filtros, en cuadrícula («Catálogo completo» desde la
 * pantalla de Series). */
export function SeriesCatalog() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const [filters, setFilters] = useState<CatalogFilters>(() => recallScreenState(FILTERS_MEMORY_KEY) ?? EMPTY_FILTERS);
  function updateFilters(next: CatalogFilters) {
    setFilters(next);
    rememberScreenState(FILTERS_MEMORY_KEY, next);
  }

  const query = useInfiniteQuery({
    queryKey: ["series", profileId, filters],
    queryFn: ({ pageParam }: { pageParam: PageParam }) => {
      const params = new URLSearchParams({ profile_id: String(profileId) });
      if (filters.genre != null) params.set("genre", String(filters.genre));
      if (filters.year != null) params.set("year", String(filters.year));
      const sort = sortParam(filters.sort);
      if (sort) params.set("sort", sort);
      if (pageParam.after != null) params.set("after", String(pageParam.after));
      if (pageParam.afterKey != null) params.set("after_key", pageParam.afterKey);
      return apiGet<SeriesPage>(`/api/series?${params.toString()}`);
    },
    initialPageParam: {} as PageParam,
    getNextPageParam: (lastPage) => (lastPage.next_cursor != null ? { after: lastPage.next_cursor, afterKey: lastPage.next_cursor_key ?? undefined } : undefined),
    enabled: profileId !== null,
  });

  const items = (query.data?.pages ?? []).flatMap((page) =>
    page.items.map((series) => ({
      id: series.id,
      title: series.clean_title,
      posterPath: series.poster_path,
      subtitle: series.year ? String(series.year) : undefined,
    })),
  );

  return (
    <div className="flex h-full flex-col gap-4 py-5 sm:gap-6 sm:py-8">
      <div className="flex flex-wrap items-center gap-3 px-4 sm:px-10">
        <button
          type="button"
          onClick={() => navigate({ to: "/series" })}
          className="flex items-center gap-1.5 rounded-full bg-neutral-900 px-3 py-1.5 text-sm text-neutral-300 outline-none hover:bg-neutral-800 hover:text-white focus-visible:ring-2 focus-visible:ring-red-600 ios:bg-white/10"
          aria-label="Volver a destacados"
        >
          <BackIcon className="h-4 w-4" />
          Destacados
        </button>
        <LargeTitle>Series: catálogo completo</LargeTitle>
      </div>
      <CatalogFilterBar kind="tv" filters={filters} onChange={updateFilters} />
      <div className="min-h-0 flex-1">
        <VirtualPosterGrid
          items={items}
          hasMore={query.hasNextPage ?? false}
          isLoadingMore={query.isFetchingNextPage}
          isInitialLoading={query.isPending}
          onLoadMore={() => query.fetchNextPage()}
          onOpen={(id) => navigate({ to: "/series/$seriesId", params: { seriesId: String(id) } })}
          emptyMessage="Nada con estos filtros. Prueba a quitar alguno, o añade un proveedor en Ajustes si el catálogo está vacío."
          favoriteKind="series"
          restoreKey={JSON.stringify(filters)}
        />
      </div>
    </div>
  );
}
