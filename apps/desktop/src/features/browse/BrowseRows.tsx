import { useNavigate } from "@tanstack/react-router";
import type { BrowseItem, BrowseRowResponse } from "../../lib/api-types";
import { Row } from "../../components/Row";
import { PosterCard } from "../../components/PosterCard";

/** Las filas de `GET /api/browse`: pósters, el Top 10 con su número y «Seguir viendo»
 * (pinchar reproduce directamente, con la barra de lo visto). */
export function BrowseRows({ rows }: { rows: BrowseRowResponse[] }) {
  const navigate = useNavigate();

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
        <Row key={row.key} title={row.title}>
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
              menuItem={item.kind === "movie" || item.kind === "series" ? { kind: item.kind, id: item.id } : undefined}
            />
          ))}
        </Row>
      ))}
    </>
  );
}
