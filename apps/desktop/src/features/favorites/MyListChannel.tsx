import type { FavoriteItem } from "../../lib/api-types";
import { ChannelLogo } from "../../components/ChannelLogo";
import { PlayIcon } from "../../components/icons";

/** Un canal de Mi lista: logo, nombre y lo que emite ahora. Tocarlo lo pone. */
export function MyListChannel({ item, editing, selected, onOpen }: { item: FavoriteItem; editing: boolean; selected: boolean; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-pressed={editing ? selected : undefined}
      className={`flex w-full items-center gap-3 rounded-xl bg-white/[0.04] p-3 text-left outline-none ring-1 transition-colors hover:bg-white/[0.08] focus-visible:ring-2 focus-visible:ring-white/70 ios:bg-ios-surface-1 ${
        selected ? "ring-red-500" : "ring-white/[0.06] ios:ring-0"
      }`}
    >
      <ChannelLogo channelId={item.item_id} name={item.title} className="h-11 w-11 rounded-lg text-base" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-semibold text-white">{item.title}</span>
        <span className="truncate text-xs text-neutral-400">{item.now_playing ? `Ahora: ${item.now_playing}` : "Sin guía ahora"}</span>
      </span>
      {editing ? (
        <span className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-bold ring-2 ${selected ? "bg-red-600 text-white ring-red-600" : "text-transparent ring-white/60"}`}>✓</span>
      ) : (
        <PlayIcon className="h-5 w-5 shrink-0 text-neutral-400" />
      )}
    </button>
  );
}
