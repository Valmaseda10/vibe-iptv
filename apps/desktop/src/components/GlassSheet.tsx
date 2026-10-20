import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

type GlassSheetProps = {
  /** Nombre accesible de la hoja (y título visible si `showTitle`). */
  title: string;
  showTitle?: boolean;
  onClose: () => void;
  children: ReactNode;
};

/** Distancia (px) que hay que arrastrar hacia abajo para cerrar la hoja. */
const DRAG_CLOSE_PX = 80;

/** Hoja inferior de cristal, como las de iOS 26 (F15): flota a unos píxeles de los bordes,
 * con asa, se cierra tocando fuera, con Escape/Atrás o arrastrándola hacia abajo. En el PC
 * (sin `html.ios`) es un panel oscuro centrado. Se pinta en `body` para que ningún
 * `transform` de quien la abre la deforme. Las filas de dentro no llevan cristal propio:
 * nunca cristal sobre cristal. */
export function GlassSheet({
  title,
  showTitle = true,
  onClose,
  children,
}: GlassSheetProps) {
  const [dragY, setDragY] = useState(0);
  const dragStart = useRef<number | null>(null);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Backspace") {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return createPortal(
    <div
      className="fixed inset-0 z-[60] flex items-end justify-center sm:items-center"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <button
        type="button"
        aria-label="Cerrar"
        tabIndex={-1}
        onClick={onClose}
        className="sheet-fade absolute inset-0 bg-black/50"
      />
      {/* La animación de entrada va en una capa y el arrastre en otra: una animación CSS
          con relleno pisa el `transform` en línea. */}
      <div className="sheet-up relative w-full max-w-md">
        <div
          className="m-2 mb-[max(0.5rem,env(safe-area-inset-bottom))] rounded-[28px] bg-neutral-900 p-2 shadow-2xl ios:glass"
          style={
            dragY > 0
              ? { transform: `translateY(${dragY}px)`, transition: "none" }
              : { transition: "transform 250ms var(--ease-spring)" }
          }
        >
          <div
            className="flex cursor-grab touch-none justify-center pb-1 pt-1.5"
            onPointerDown={(e) => {
              dragStart.current = e.clientY;
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (dragStart.current != null)
                setDragY(Math.max(0, e.clientY - dragStart.current));
            }}
            onPointerUp={() => {
              dragStart.current = null;
              if (dragY > DRAG_CLOSE_PX) onClose();
              else setDragY(0);
            }}
            onPointerCancel={() => {
              dragStart.current = null;
              setDragY(0);
            }}
          >
            <div className="h-[5px] w-9 rounded-full bg-white/30" />
          </div>
          {showTitle && (
            <p className="truncate px-4 pb-2 pt-1 text-center text-[15px] font-semibold text-white">
              {title}
            </p>
          )}
          {children}
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** Clase de una fila de acción dentro de una `GlassSheet`. */
export const SHEET_ROW =
  "flex w-full items-center gap-3 rounded-2xl px-4 py-3.5 text-left text-[17px] text-white outline-none active:bg-white/10 focus-visible:bg-white/10 disabled:opacity-50";
