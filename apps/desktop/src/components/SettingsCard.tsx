import type { ReactNode } from "react";

/** Grupo de ajustes (2026-11-02): un título pequeño encima y una tarjeta con las filas
 * separadas por una línea fina, como en los Ajustes de Windows 11 y de iOS. */
export function SettingsCard({ title, footer, children }: { title?: string; footer?: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      {title && <h3 className="px-1 text-[13px] font-semibold uppercase tracking-wide text-neutral-400 ios:normal-case ios:tracking-normal ios:text-ios-label-2">{title}</h3>}
      <div className="divide-y divide-white/[0.06] overflow-hidden rounded-xl bg-white/[0.04] ring-1 ring-white/[0.06] ios:rounded-[18px] ios:bg-ios-surface-1 ios:ring-0">
        {children}
      </div>
      {footer && <div className="px-1 text-[13px] leading-snug text-neutral-500 ios:text-ios-label-2">{footer}</div>}
    </section>
  );
}
