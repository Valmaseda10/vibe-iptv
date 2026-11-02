import type { ReactNode } from "react";

type SettingsRowProps = {
  title: ReactNode;
  description?: ReactNode;
  /** Icono a la izquierda (opcional). */
  icon?: ReactNode;
  /** El control a la derecha: interruptor, selector, botón... */
  children?: ReactNode;
  /** Controles que no caben a la derecha van debajo, a todo el ancho. */
  stacked?: boolean;
  /** El control va siempre a la derecha, también en el iPhone (interruptores). */
  inline?: boolean;
};

/** Una fila de ajustes: título, explicación corta y el control a la derecha. Mismo
 * formato en toda la pantalla de Ajustes. */
export function SettingsRow({ title, description, icon, children, stacked = false, inline = false }: SettingsRowProps) {
  return (
    <div className={`flex gap-3 px-4 py-3.5 ${stacked ? "flex-col" : inline ? "flex-row items-center gap-4" : "flex-col sm:flex-row sm:items-center sm:gap-4"}`}>
      <div className="flex min-w-0 flex-1 items-start gap-3">
        {icon && <span className="mt-0.5 shrink-0 text-neutral-300">{icon}</span>}
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-[15px] font-medium text-white">{title}</span>
          {description && <span className="text-[13px] leading-snug text-neutral-400 ios:text-ios-label-2">{description}</span>}
        </div>
      </div>
      {children && <div className={`flex shrink-0 flex-wrap items-center gap-2 ${stacked ? "" : "sm:justify-end"}`}>{children}</div>}
    </div>
  );
}
