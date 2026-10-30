import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { apiDelete } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { BrowseItem, BrowseRowResponse } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { EMPTY_FILTERS, type CatalogFilters } from "../../components/CatalogFilterBar";
import { rememberScreenState } from "../../lib/screenMemory";
import { PosterCard } from "../../components/PosterCard";

/** Las filas de `GET /api/browse`: pósters, el Top 10 con su número y «Continuar viendo»
 * (pinchar reproduce directamente, con la barra de lo visto). */
/** «Ver todo» de una fila: el catálogo completo con el filtro que corresponde (el género de
 * la fila, lo mejor valorado o lo más reciente). `null` si la fila no tiene equivalente. */
function catalogLink(key: string, page: "movies" | "series" | "home"): { kind: "movies" | "series"; filters: CatalogFilters } | null {
  const match = /^(?:(movie|series)-)?(genre-(\d+)|acclaimed|new)$/.exec(key);
  if (!match) return null;
  const kind = match[1] === "series" || (!match[1] && page === "series") ? "series" : match[1] === "movie" || page === "movies" ? "movies" : null;
  if (!kind) return null;
  if (match[3]) return { kind, filters: { ...EMPTY_FILTERS, genre: Number(match[3]) } };
  return { kind, filters: { ...EMPTY_FILTERS, sort: match[2] === "acclaimed" ? "rating" : "recent" } };
}

export function BrowseRows({ rows, page }: { rows: BrowseRowResponse[]; page: "movies" | "series" | "home" }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { profileId } = useActiveProfile();

  // Quitar algo de «Continuar viendo», igual que en Inicio.
  async function removeFromContinue(item: BrowseItem) {
    await apiDelete(`/api/playback/progress?profile_id=${profileId}&item_kind=${item.kind}&item_id=${item.id}`).catch(() => {});
    queryClient.invalidateQueries({ queryKey: ["browse"] });
    queryClient.invalidateQueries({ queryKey: ["home"] });
  }

  function rowAction(key: string) {
    const link = catalogLink(key, page);
    if (!link) return undefined;
    return {
      label: "Ver todo ›",
      onClick: () => {
        // El catálogo abre con este filtro (lo recuerda como si se hubiera elegido allí).
        rememberScreenState(`${link.kind}:filters`, link.filters);
        if (link.kind === "series") navigate({ to: "/series", search: { view: "all" } });
        else navigate({ to: "/movies", search: { view: "all" } });
      },
    };
  }

  function open(item: BrowseItem, play: boolean) {
    if (item.kind === "episode") {
      navigate({
        to: "/player/$kind/$id",
        params: { kind: "episode", id: String(item.id) },
        search: item.series_id != null ? { seriesId: String(item.series_id) } : {},
      });
    } else if (item.kind === "series") {
      navigate({ to: "/series/$seriesId", params: { seriesId: String(item.id) } });
    } else if (play) {
      navigate({ to: "/player/$kind/$id", params: { kind: "movie", id: String(item.id) } });
    } else {
      navigate({ to: "/movies/$movieId", params: { movieId: String(item.id) } });
    }
  }

  return (
    <>
      {rows.map((row) => (
        <Row key={row.key} title={row.title} action={rowAction(row.key)}>
          {row.items.map((item, index) => (
            <PosterCard
              key={`${row.key}-${item.kind}-${item.id}`}
              title={item.clean_title}
              posterPath={item.poster_path}
              subtitle={item.subtitle ?? undefined}
              isNew={item.is_new}
              rank={row.style === "top10" ? index + 1 : undefined}
              progressPercent={item.progress != null ? item.progress * 100 : undefined}
              onOpen={() => open(item, row.style === "continue")}
              onRemove={row.style === "continue" && item.progress != null ? () => void removeFromContinue(item) : undefined}
              menuItem={item.kind === "movie" || item.kind === "series" ? { kind: item.kind, id: item.id } : undefined}
              preview={
                item.kind === "movie" || item.kind === "series"
                  ? {
                      kind: item.kind,
                      id: item.id,
                      year: item.year,
                      rating: item.rating,
                      genres: item.genres,
                      backdropPath: item.backdrop_path,
                      onPlay: item.kind === "movie" ? () => open(item, true) : undefined,
                    }
                  : undefined
              }
            />
          ))}
        </Row>
      ))}
    </>
  );
}
