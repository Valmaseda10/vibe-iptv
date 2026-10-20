import { IS_IOS } from "../lib/platform";

/** Botón secundario de las fichas (Mi lista, Marcar vista, Tráiler): en el iPhone, icono
 * con la etiqueta debajo, como en Netflix; en pantallas anchas, botón con borde. En iOS
 * (F15) son losetas redondeadas: superficies sólidas, el cristal es solo para lo que flota. */
export const DETAIL_ACTION = IS_IOS
  ? "flex flex-col items-center gap-1 rounded-2xl bg-white/[0.08] px-2 py-2.5 text-xs font-medium text-white outline-none active:bg-white/15 sm:flex-row sm:gap-2 sm:rounded-full sm:px-5 sm:py-3 sm:text-sm"
  : "flex flex-col items-center gap-1 rounded-lg px-2 py-2 text-xs text-neutral-200 outline-none sm:flex-row sm:gap-2 sm:border sm:border-neutral-700 sm:px-4 sm:py-3 sm:text-sm sm:hover:bg-neutral-900";

/** Acción principal de una ficha ("▶ Reproducir", "▶ Continuar T1·E2"). En iOS, cápsula
 * blanca en todos los tamaños: una sola acción principal por vista. */
export const PRIMARY_ACTION = IS_IOS
  ? "flex w-full items-center justify-center gap-2 rounded-full bg-white px-7 py-3.5 text-[17px] font-semibold text-black outline-none active:bg-neutral-300 sm:w-auto"
  : "flex w-full items-center justify-center gap-2 rounded-lg bg-white px-6 py-3 font-semibold text-black outline-none hover:bg-neutral-200 sm:w-auto sm:bg-red-600 sm:text-white sm:hover:bg-red-500";
