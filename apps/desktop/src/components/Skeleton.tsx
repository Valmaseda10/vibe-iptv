import type { CSSProperties } from "react";

/** Siluetas de carga (F17): la forma de lo que va a aparecer, con un brillo que pasa, en
 * vez de «Cargando…». La misma familia en todas las pantallas, en el PC y en el iPhone. */

export function SkeletonBlock({ className = "", style }: { className?: string; style?: CSSProperties }) {
  return <div aria-hidden className={`skeleton rounded-lg ${className}`} style={style} />;
}

/** Una fila de portadas, con su título. */
export function SkeletonRow({ count = 7 }: { count?: number }) {
  return (
    <section className="flex flex-col gap-3" aria-hidden>
      <SkeletonBlock className="mx-4 h-5 w-48 sm:mx-10" />
      <div className="flex gap-3 overflow-hidden px-4 sm:gap-4 sm:px-10">
        {Array.from({ length: count }, (_, i) => (
          <SkeletonBlock key={i} className="aspect-[2/3] w-32 shrink-0 sm:w-40" />
        ))}
      </div>
    </section>
  );
}

/** Una rejilla de portadas. */
export function SkeletonGrid({ count = 12 }: { count?: number }) {
  return (
    <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8" aria-hidden>
      {Array.from({ length: count }, (_, i) => (
        <SkeletonBlock key={i} className="aspect-[2/3] w-full" />
      ))}
    </div>
  );
}

/** Una lista de filas (canales, ajustes, equipos). */
export function SkeletonList({ count = 6, withIcon = true }: { count?: number; withIcon?: boolean }) {
  return (
    <div className="flex flex-col gap-3" aria-hidden>
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="flex items-center gap-3">
          {withIcon && <SkeletonBlock className="h-10 w-10 shrink-0 rounded-full" />}
          <div className="flex flex-1 flex-col gap-2">
            <SkeletonBlock className="h-3.5" style={{ width: `${60 + ((i * 17) % 30)}%` }} />
            <SkeletonBlock className="h-3 w-1/3" />
          </div>
        </div>
      ))}
    </div>
  );
}

/** La ficha de una película o serie: fondo, título, datos, sinopsis y botones. */
export function SkeletonDetail() {
  return (
    <div className="relative min-h-full" aria-busy aria-label="Cargando la ficha">
      <SkeletonBlock className="absolute inset-x-0 top-0 h-[60vh] rounded-none opacity-60" />
      <div className="absolute inset-x-0 top-0 h-[60vh] bg-gradient-to-t from-neutral-950 via-neutral-950/60 to-transparent ios:from-black" />
      <div className="relative flex flex-col gap-4 px-4 pt-[32vh] sm:px-10">
        <SkeletonBlock className="h-12 w-3/4 max-w-xl" />
        <SkeletonBlock className="h-4 w-56" />
        <div className="flex max-w-2xl flex-col gap-2">
          <SkeletonBlock className="h-3.5 w-full" />
          <SkeletonBlock className="h-3.5 w-11/12" />
          <SkeletonBlock className="h-3.5 w-2/3" />
        </div>
        <div className="flex gap-3 pt-2">
          <SkeletonBlock className="h-12 w-40" />
          <SkeletonBlock className="h-12 w-28" />
          <SkeletonBlock className="h-12 w-28" />
        </div>
      </div>
    </div>
  );
}

/** Al abrir la app, mientras se sabe qué cuenta y perfil hay. */
export function AppSplash() {
  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-4 bg-neutral-950 ios:bg-black" aria-busy aria-label="Abriendo VIBE IPTV">
      <div className="splash-logo flex items-center gap-2 text-2xl font-bold tracking-wide text-white">
        <span className="h-3.5 w-3.5 rounded-full bg-red-600" />
        VIBE IPTV
      </div>
    </div>
  );
}
