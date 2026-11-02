import { imagePath } from "../../lib/api-client";
import type { FavoriteItem } from "../../lib/api-types";
import { CloseIcon } from "../../components/icons";

type Props = {
  item: FavoriteItem;
  base: string | undefined;
  editing: boolean;
  selected: boolean;
  onOpen: () => void;
  onRemove: () => void;
};

/** Una película o serie de Mi lista: portada con su estado (episodios nuevos, visto, lo que
 * va visto) y, al pasar el ratón, quitarla. En «Editar», tocarla la marca para quitar. */
export function MyListCard({ item, base, editing, selected, onOpen, onRemove }: Props) {
  const poster = imagePath(base, item.poster_path, "w342");
  return (
    <div className="group relative flex flex-col gap-2">
      <button
        type="button"
        data-card
        onClick={onOpen}
        aria-label={item.title}
        aria-pressed={editing ? selected : undefined}
        className={`relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-neutral-900 outline-none ring-offset-2 ring-offset-neutral-950 transition-transform duration-200 focus-visible:ring-2 focus-visible:ring-white ios:rounded-xl ${
          editing ? (selected ? "scale-[0.94] ring-2 ring-red-500" : "") : "group-hover:scale-[1.03]"
        }`}
      >
        {poster ? (
          <img src={poster} alt="" loading="lazy" className="h-full w-full object-cover" />
        ) : (
          <span className="flex h-full w-full items-center justify-center p-3 text-center text-sm text-neutral-400">{item.title}</span>
        )}
        {item.is_new && <span className="absolute left-2 top-2 rounded bg-red-600 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-white shadow">Nuevo episodio</span>}
        {item.watched && !editing && (
          <span className="absolute right-2 top-2 flex h-6 w-6 items-center justify-center rounded-full bg-black/70 text-xs font-bold text-emerald-400 ring-1 ring-white/20" title="Vista">
            ✓
          </span>
        )}
        {item.progress != null && item.progress > 0 && (
          <span className="absolute inset-x-0 bottom-0 h-1 bg-white/20">
            <span className="block h-full bg-red-600" style={{ width: `${Math.max(4, item.progress * 100)}%` }} />
          </span>
        )}
        {editing && (
          <span
            className={`absolute right-2 top-2 flex h-6 w-6 items-center justify-center rounded-full text-xs font-bold ring-2 ${
              selected ? "bg-red-600 text-white ring-red-600" : "bg-black/50 text-transparent ring-white/70"
            }`}
          >
            ✓
          </span>
        )}
      </button>
      {!editing && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Quitar ${item.title} de Mi lista`}
          className="absolute right-2 top-2 hidden h-7 w-7 items-center justify-center rounded-full bg-black/70 text-white opacity-0 outline-none ring-1 ring-white/20 transition-opacity hover:bg-black focus-visible:opacity-100 group-hover:opacity-100 desk:flex"
        >
          <CloseIcon className="h-3.5 w-3.5" />
        </button>
      )}
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm font-medium text-neutral-100">{item.title}</span>
        <span className="text-xs text-neutral-500">
          {[item.item_kind === "series" ? "Serie" : "Película", item.year].filter(Boolean).join(" · ")}
        </span>
      </div>
    </div>
  );
}
