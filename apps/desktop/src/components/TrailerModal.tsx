import { useEffect } from "react";

/** Modal a pantalla completa con el tráiler embebido de YouTube (`youtube-nocookie.com`:
 * no deja cookies de seguimiento hasta que se le da a reproducir, lo más parecido a "no
 * hay ningún tráfico de más" sin dejar de ser un embed real). No tiene nada que ver con
 * el reproductor de la app ni con el proveedor IPTV — no pasa por `StreamGuard` porque no
 * es un stream del proveedor (regla R1, ver `catalog::tmdb::movie_trailer_key`). */
export function TrailerModal({ youtubeKey, title, onClose }: { youtubeKey: string; title: string; onClose: () => void }) {
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/90 p-6" onClick={onClose}>
      <div className="relative w-full max-w-4xl" onClick={(e) => e.stopPropagation()}>
        <button
          type="button"
          onClick={onClose}
          autoFocus
          aria-label="Cerrar tráiler"
          className="absolute -top-10 right-0 text-2xl text-neutral-300 outline-none hover:text-white focus-visible:text-white"
        >
          ✕
        </button>
        <div className="aspect-video w-full overflow-hidden rounded-lg bg-black shadow-2xl">
          <iframe
            src={`https://www.youtube-nocookie.com/embed/${youtubeKey}?autoplay=1`}
            title={`Tráiler de ${title}`}
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
            allowFullScreen
            className="h-full w-full"
          />
        </div>
      </div>
    </div>
  );
}
