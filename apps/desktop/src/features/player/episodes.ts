import type { EpisodeSummary } from "../../lib/api-types";

/** El episodio que va después de `current`: el siguiente de su temporada o, al acabarla,
 * el primero de la temporada siguiente. */
export function findNextEpisode(all: EpisodeSummary[], current: EpisodeSummary): EpisodeSummary | undefined {
  const sameSeason = all.filter((e) => e.season === current.season).sort((a, b) => a.number - b.number);
  const inSeasonNext = sameSeason.find((e) => e.number > current.number);
  if (inSeasonNext) return inSeasonNext;

  const seasons = [...new Set(all.map((e) => e.season))].sort((a, b) => a - b);
  const nextSeason = seasons.find((s) => s > current.season);
  if (nextSeason == null) return undefined;
  return all.filter((e) => e.season === nextSeason).sort((a, b) => a.number - b.number)[0];
}
