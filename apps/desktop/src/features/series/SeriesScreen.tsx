import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { SeriesPage } from "../../lib/api-types";
import { VirtualPosterGrid } from "../../components/VirtualPosterGrid";

export function SeriesScreen() {
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const query = useInfiniteQuery({
    queryKey: ["series", profileId],
    queryFn: ({ pageParam }: { pageParam: number | undefined }) =>
      apiGet<SeriesPage>(`/api/series?profile_id=${profileId}${pageParam ? `&after=${pageParam}` : ""}`),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
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
    <div className="flex h-full flex-col gap-6 py-8">
      <h1 className="px-6 text-3xl font-semibold sm:px-10">Series</h1>
      <div className="min-h-0 flex-1">
        <VirtualPosterGrid
          items={items}
          hasMore={query.hasNextPage ?? false}
          isLoadingMore={query.isFetchingNextPage}
          isInitialLoading={query.isPending}
          onLoadMore={() => query.fetchNextPage()}
          onOpen={(id) => navigate({ to: "/series/$seriesId", params: { seriesId: String(id) } })}
          emptyMessage="Todavía no hay series sincronizadas. Añade un proveedor en Ajustes."
          favoriteKind="series"
        />
      </div>
    </div>
  );
}
