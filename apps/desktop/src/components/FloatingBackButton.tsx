import { createPortal } from "react-dom";
import { IS_IOS } from "../lib/platform";

/** "Atrás" de las fichas en el iPhone (F15): botón redondo de cristal que flota arriba a
 * la izquierda sobre el fondo, como en la app TV. Solo en iOS sin barra lateral; en el
 * resto se usa el "← Volver" de texto de siempre. */
export function FloatingBackButton({ onClick }: { onClick: () => void }) {
  if (!IS_IOS) return null;
  // En el <body>: un `transform` de un antecesor (la animación de entrada de la pantalla)
  // convertiría `fixed` en relativo a él.
  return createPortal(
    <button
      type="button"
      onClick={onClick}
      aria-label="Volver"
      className="glass glass-pressable fixed left-4 top-[calc(env(safe-area-inset-top)+8px)] z-40 flex h-11 w-11 items-center justify-center rounded-full text-white outline-none desk:hidden"
    >
      <svg viewBox="0 0 24 24" className="h-6 w-6" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
        <path d="M15 18l-6-6 6-6" />
      </svg>
    </button>,
    document.body,
  );
}
