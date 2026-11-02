import { GlassSheet } from "./GlassSheet";
import { BrandMark } from "./BrandMark";
import { RELEASE_NOTES } from "../lib/releaseNotes";

/** «Novedades de la versión X»: lo que trae esta versión, en una hoja. */
export function WhatsNewSheet({ onClose }: { onClose: () => void }) {
  return (
    <GlassSheet title={`Novedades de la versión ${RELEASE_NOTES.version}`} showTitle={false} onClose={onClose}>
      <div className="flex max-h-[80vh] flex-col gap-5 overflow-y-auto px-4 pb-4 pt-2">
        <div className="flex items-center gap-3">
          <BrandMark className="h-10 w-10" />
          <div className="flex flex-col">
            <span className="text-[13px] font-medium uppercase tracking-wide text-neutral-400">Novedades</span>
            <h2 className="text-xl font-bold text-white">Versión {RELEASE_NOTES.version}</h2>
          </div>
        </div>
        <ul className="flex flex-col gap-3">
          {RELEASE_NOTES.notes.map((note) => (
            <li key={note} className="flex gap-3 text-[15px] leading-snug text-neutral-200">
              <span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-gradient-to-br from-orange-400 to-pink-600" aria-hidden />
              <span>{note}</span>
            </li>
          ))}
        </ul>
        <button
          type="button"
          autoFocus
          onClick={onClose}
          className="rounded-xl bg-white py-3 text-[15px] font-semibold text-black outline-none hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-white/70 ios:rounded-full"
        >
          Entendido
        </button>
      </div>
    </GlassSheet>
  );
}
