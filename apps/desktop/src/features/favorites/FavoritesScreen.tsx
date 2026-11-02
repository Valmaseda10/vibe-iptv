import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { apiGet, useApiBase } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { useFavorites } from "../../lib/favorites";
import type { FavoriteItem } from "../../lib/api-types";
import { LargeTitle } from "../../components/LargeTitle";
import { SkeletonGrid } from "../../components/Skeleton";
import { SegmentedChoice } from "../../components/SegmentedChoice";
import { HeartIcon } from "../../components/icons";
import { ProfilePanel } from "./ProfilePanel";
import { MyListCard } from "./MyListCard";
import { MyListChannel } from "./MyListChannel";

type Filter = "all" | "movie" | "series" | "channel";
type Sort = "recent" | "title" | "year";

const SORTS: readonly { value: Sort; label: string }[] = [
  { value: "recent", label: "Recientes" },
  { value: "title", label: "A–Z" },
  { value: "year", label: "Año" },
];

const key = (item: FavoriteItem) => `${item.item_kind}:${item.item_id}`;

/** «Mi lista» (rediseño 2026-11-02): lo que el perfil ha marcado con el corazón, con filtro
 * como el de Inicio, orden, estado de cada título (empezado, visto, episodios nuevos), los
 * canales con lo que emiten ahora y «Editar» para quitar varios a la vez. En el iPhone,
 * arriba, el perfil con Ajustes, «Tu año», «Sorpréndeme» y «Cambiar de perfil». */
export function FavoritesScreen() {
  const navigate = useNavigate();
  const { toggleFavorite } = useFavorites();
  const { profileId } = useActiveProfile();
  const { data: base } = useApiBase();
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("recent");
  const [editing, setEditing] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const { data, isPending } = useQuery({
    queryKey: ["favorites", profileId],
    queryFn: () => apiGet<FavoriteItem[]>(`/api/favorites?profile_id=${profileId}`),
    enabled: profileId !== null,
  });

  const all = useMemo(() => (data ?? []).filter((f) => f.title !== ""), [data]);
  const counts = useMemo(
    () => ({
      all: all.length,
      movie: all.filter((f) => f.item_kind === "movie").length,
      series: all.filter((f) => f.item_kind === "series").length,
      channel: all.filter((f) => f.item_kind === "channel").length,
    }),
    [all],
  );
  const sorted = useMemo(() => {
    const list = [...all];
    if (sort === "title") list.sort((a, b) => a.title.localeCompare(b.title, "es"));
    else if (sort === "year") list.sort((a, b) => (b.year ?? 0) - (a.year ?? 0));
    else list.sort((a, b) => b.added_at - a.added_at);
    return list;
  }, [all, sort]);
  const titles = sorted.filter((f) => f.item_kind !== "channel" && (filter === "all" || f.item_kind === filter));
  const channels = sorted.filter((f) => f.item_kind === "channel" && (filter === "all" || filter === "channel"));

  const filters = (
    [
      { value: "all", label: "Todo" },
      { value: "movie", label: "Películas" },
      { value: "series", label: "Series" },
      { value: "channel", label: "Canales" },
    ] as const
  )
    .filter((f) => f.value === "all" || counts[f.value] > 0)
    .map((f) => ({ value: f.value as Filter, label: `${f.label} ${counts[f.value]}` }));

  function open(item: FavoriteItem) {
    if (editing) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(key(item))) next.delete(key(item));
        else next.add(key(item));
        return next;
      });
      return;
    }
    if (item.item_kind === "movie") navigate({ to: "/movies/$movieId", params: { movieId: String(item.item_id) } });
    else if (item.item_kind === "series") navigate({ to: "/series/$seriesId", params: { seriesId: String(item.item_id) } });
    else navigate({ to: "/player/$kind/$id", params: { kind: "channel", id: String(item.item_id) } });
  }

  function removeSelected() {
    for (const item of all) if (selected.has(key(item))) toggleFavorite(item.item_kind, item.item_id, false);
    setSelected(new Set());
    setEditing(false);
  }

  return (
    <div className="flex flex-col gap-6 px-4 py-5 sm:gap-8 sm:px-10 sm:py-8">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="flex flex-col gap-1">
          <LargeTitle>Mi lista</LargeTitle>
          {counts.all > 0 && (
            <p className="text-sm text-neutral-400">
              {counts.all} {counts.all === 1 ? "guardado" : "guardados"}
            </p>
          )}
        </div>
        {counts.all > 0 && (
          <div className="flex items-center gap-2">
            {editing && (
              <button
                type="button"
                onClick={removeSelected}
                disabled={selected.size === 0}
                className="rounded-full bg-red-600 px-4 py-2 text-sm font-semibold text-white outline-none hover:bg-red-500 focus-visible:ring-2 focus-visible:ring-white/70 disabled:opacity-40"
              >
                Quitar{selected.size > 0 ? ` (${selected.size})` : ""}
              </button>
            )}
            <button
              type="button"
              onClick={() => {
                setEditing((e) => !e);
                setSelected(new Set());
              }}
              className="rounded-full bg-white/10 px-4 py-2 text-sm font-medium text-white outline-none hover:bg-white/15 focus-visible:ring-2 focus-visible:ring-white/70"
            >
              {editing ? "Hecho" : "Editar"}
            </button>
          </div>
        )}
      </div>

      <ProfilePanel />

      {isPending && <SkeletonGrid count={12} />}

      {!isPending && counts.all === 0 && (
        <div className="flex flex-col items-center gap-4 rounded-2xl bg-white/[0.03] px-6 py-14 text-center ring-1 ring-white/[0.05]">
          <span className="flex h-16 w-16 items-center justify-center rounded-full bg-white/[0.06] text-neutral-300">
            <HeartIcon className="h-8 w-8" />
          </span>
          <div className="flex flex-col gap-1">
            <p className="text-lg font-semibold text-white">Tu lista está vacía</p>
            <p className="max-w-sm text-sm text-neutral-400">El corazón de cada película, serie o canal lo guarda aquí para verlo cuando quieras.</p>
          </div>
          <Link to="/" className="rounded-full bg-white px-5 py-2.5 text-sm font-semibold text-black outline-none hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-white/70">
            Buscar qué ver
          </Link>
        </div>
      )}

      {counts.all > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <SegmentedChoice label="Qué ver" options={filters} value={filter} onChange={setFilter} />
          {filter !== "channel" && <SegmentedChoice label="Ordenar" options={SORTS} value={sort} onChange={setSort} />}
        </div>
      )}

      {titles.length > 0 && (
        <ul className="grid grid-cols-3 gap-x-3 gap-y-5 sm:grid-cols-4 sm:gap-x-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 2xl:grid-cols-8">
          {titles.map((item) => (
            <li key={key(item)}>
              <MyListCard
                item={item}
                base={base}
                editing={editing}
                selected={selected.has(key(item))}
                onOpen={() => open(item)}
                onRemove={() => toggleFavorite(item.item_kind, item.item_id, false)}
              />
            </li>
          ))}
        </ul>
      )}

      {channels.length > 0 && (
        <section className="flex flex-col gap-3">
          {filter === "all" && <h2 className="text-lg font-semibold text-neutral-200 ios:text-[20px] ios:font-bold ios:text-white">Canales</h2>}
          <ul className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
            {channels.map((item) => (
              <li key={key(item)}>
                <MyListChannel item={item} editing={editing} selected={selected.has(key(item))} onOpen={() => open(item)} />
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
