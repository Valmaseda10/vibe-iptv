import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiGet, imagePath, useApiBase } from "../../lib/api-client";
import type { BrowseResponse, FavoriteItem } from "../../lib/api-types";
import { AVATAR_COLORS, AVATAR_GRADIENTS, AVATAR_ICONS, CHARACTER_STYLES, backgroundCss, type Avatar, type CharacterStyle } from "../../lib/avatar";
import { FLUENT_AVATARS } from "../../lib/fluentAvatars.generated";
import { AvatarImage } from "../../components/AvatarImage";
import { SegmentedChoice } from "../../components/SegmentedChoice";

type Tab = "fluent" | "character" | "title" | "classic";
const TABS: readonly { value: Tab; label: string }[] = [
  { value: "fluent", label: "Iconos 3D" },
  { value: "character", label: "Personajes" },
  { value: "title", label: "Películas y series" },
  { value: "classic", label: "Clásicos" },
];
const FLUENT_GROUPS = [
  { value: "personajes", label: "Personajes" },
  { value: "cine", label: "Cine y TV" },
  { value: "animales", label: "Animales" },
  { value: "caras", label: "Caras" },
  { value: "mas", label: "Más" },
] as const;
const CHARACTERS_PER_PAGE = 18;

function randomSeeds(count: number): string[] {
  return Array.from({ length: count }, () => Math.random().toString(36).slice(2, 10));
}

/** Elegir la imagen del perfil (2026-11-02): iconos 3D, personajes ilustrados (cada «Otros»
 * trae nuevos), una película o serie del catálogo, o los emoji de antes; y el fondo. */
export function AvatarPicker({ value, name, catalogProfileId, onChange }: { value: Avatar; name: string; catalogProfileId: number | null; onChange: (avatar: Avatar) => void }) {
  const [tab, setTab] = useState<Tab>(value.kind === "character" ? "character" : value.kind === "title" ? "title" : value.kind === "fluent" ? "fluent" : "classic");
  const [group, setGroup] = useState<(typeof FLUENT_GROUPS)[number]["value"]>("personajes");
  const [style, setStyle] = useState<CharacterStyle>(value.kind === "character" ? value.style : "lorelei");
  const [seeds, setSeeds] = useState(() => randomSeeds(CHARACTERS_PER_PAGE));
  const bg = value.kind === "title" ? (AVATAR_GRADIENTS[0]?.id ?? "#dc2626") : value.bg;

  const tile = (selected: boolean) =>
    `relative aspect-square w-full overflow-hidden rounded-xl outline-none ring-offset-2 ring-offset-neutral-950 transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-white ${selected ? "ring-2 ring-white" : ""}`;

  return (
    <div className="flex flex-col gap-4">
      <SegmentedChoice label="Tipo de imagen" options={TABS} value={tab} onChange={setTab} />

      {tab === "fluent" && (
        <>
          <div className="flex flex-wrap gap-1.5">
            {FLUENT_GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                onClick={() => setGroup(g.value)}
                aria-pressed={group === g.value}
                className={`rounded-full px-3 py-1 text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-white/70 ${group === g.value ? "bg-white/20 text-white" : "text-neutral-400 hover:text-white"}`}
              >
                {g.label}
              </button>
            ))}
          </div>
          <div className="grid grid-cols-5 gap-2 sm:grid-cols-8">
            {FLUENT_AVATARS.filter((a) => a.category === group).map((a) => {
              const avatar: Avatar = { kind: "fluent", slug: a.slug, bg };
              const selected = value.kind === "fluent" && value.slug === a.slug;
              return (
                <button key={a.slug} type="button" aria-label={a.label} aria-pressed={selected} onClick={() => onChange(avatar)} className={tile(selected)}>
                  <AvatarImage avatar={avatar} name={name} className="h-full w-full rounded-xl" />
                </button>
              );
            })}
          </div>
        </>
      )}

      {tab === "character" && (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <SegmentedChoice label="Estilo" options={CHARACTER_STYLES.map((s) => ({ value: s.id, label: s.label }))} value={style} onChange={setStyle} />
            <button
              type="button"
              onClick={() => setSeeds(randomSeeds(CHARACTERS_PER_PAGE))}
              className="rounded-full bg-white/10 px-3.5 py-1.5 text-[13px] font-medium text-white outline-none hover:bg-white/15 focus-visible:ring-2 focus-visible:ring-white/70"
            >
              ↻ Otros
            </button>
          </div>
          <div className="grid grid-cols-4 gap-2 sm:grid-cols-6">
            {(value.kind === "character" && value.style === style && !seeds.includes(value.seed) ? [value.seed, ...seeds.slice(1)] : seeds).map((seed) => {
              const avatar: Avatar = { kind: "character", style, seed, bg };
              const selected = value.kind === "character" && value.style === style && value.seed === seed;
              return (
                <button key={seed} type="button" aria-label={`Personaje ${seed}`} aria-pressed={selected} onClick={() => onChange(avatar)} className={tile(selected)}>
                  <AvatarImage avatar={avatar} name={name} className="h-full w-full rounded-xl" />
                </button>
              );
            })}
          </div>
        </>
      )}

      {tab === "title" && <TitlePicker profileId={catalogProfileId} value={value} onChange={onChange} />}

      {tab === "classic" && (
        <div className="grid grid-cols-6 gap-2 sm:grid-cols-9">
          {[null, ...AVATAR_ICONS].map((emoji) => {
            const avatar: Avatar = emoji ? { kind: "emoji", emoji, bg } : { kind: "initial", bg };
            const selected = emoji ? value.kind === "emoji" && value.emoji === emoji : value.kind === "initial";
            return (
              <button key={emoji ?? "inicial"} type="button" aria-label={emoji ? `Icono ${emoji}` : "Inicial del nombre"} aria-pressed={selected} onClick={() => onChange(avatar)} className={tile(selected)}>
                <AvatarImage avatar={avatar} name={name || "?"} className="h-full w-full rounded-xl text-2xl" />
              </button>
            );
          })}
        </div>
      )}

      {value.kind !== "title" && (
        <div className="flex flex-col gap-2">
          <span className="text-[13px] font-medium text-neutral-400">Fondo</span>
          <div className="flex flex-wrap gap-2">
            {[...AVATAR_GRADIENTS.map((g) => g.id), ...AVATAR_COLORS].map((candidate) => (
              <button
                key={candidate}
                type="button"
                aria-label={`Fondo ${candidate.replace("g:", "degradado ")}`}
                aria-pressed={value.bg === candidate}
                onClick={() => onChange({ ...value, bg: candidate })}
                className={`h-8 w-8 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-white ${value.bg === candidate ? "ring-2 ring-white ring-offset-2 ring-offset-neutral-950" : ""}`}
                style={{ background: backgroundCss(candidate) }}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/** Imágenes de películas y series: las de Mi lista y las de Inicio de un perfil (el que se
 * edita; uno nuevo, las del primero). */
function TitlePicker({ profileId, value, onChange }: { profileId: number | null; value: Avatar; onChange: (avatar: Avatar) => void }) {
  const { data: base } = useApiBase();
  const favorites = useQuery({
    queryKey: ["favorites", profileId],
    queryFn: () => apiGet<FavoriteItem[]>(`/api/favorites?profile_id=${profileId}`),
    enabled: profileId !== null,
  });
  const home = useQuery({
    queryKey: ["avatar-titles", profileId],
    queryFn: () => apiGet<BrowseResponse>(`/api/browse/home?profile_id=${profileId}&seed=7`),
    enabled: profileId !== null,
    staleTime: Infinity,
  });
  const posters = useMemo(() => {
    const seen = new Set<string>();
    const out: { path: string; title: string }[] = [];
    const add = (path: string | null | undefined, title: string) => {
      // Solo las de TMDB (rutas «/xxx.jpg»): las del proveedor no se ven en otros aparatos.
      if (!path || !path.startsWith("/") || seen.has(path)) return;
      seen.add(path);
      out.push({ path, title });
    };
    for (const f of favorites.data ?? []) if (f.item_kind !== "channel") add(f.poster_path, f.title);
    for (const row of home.data?.rows ?? []) for (const item of row.items) if (item.kind === "movie" || item.kind === "series") add(item.poster_path, item.clean_title);
    return out.slice(0, 42);
  }, [favorites.data, home.data]);

  if (profileId === null) return <p className="text-sm text-neutral-400">Elige primero un perfil para ver tus películas y series.</p>;
  return (
    <div className="grid grid-cols-4 gap-2 sm:grid-cols-7">
      {posters.map(({ path, title }) => {
        const selected = value.kind === "title" && value.path === path;
        const src = imagePath(base, path, "w185");
        return (
          <button
            key={path}
            type="button"
            aria-label={title}
            aria-pressed={selected}
            onClick={() => onChange({ kind: "title", path })}
            className={`relative aspect-square overflow-hidden rounded-xl outline-none ring-offset-2 ring-offset-neutral-950 transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-white ${selected ? "ring-2 ring-white" : ""}`}
          >
            {src && <img src={src} alt="" loading="lazy" className="h-full w-full object-cover object-[50%_25%]" />}
          </button>
        );
      })}
      {posters.length === 0 && <p className="col-span-full text-sm text-neutral-400">Todavía no hay portadas: añade algo a Mi lista o espera a que se carguen las de Inicio.</p>}
    </div>
  );
}
