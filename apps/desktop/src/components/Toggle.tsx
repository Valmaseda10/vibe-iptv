/** Interruptor de encendido/apagado, el mismo en todos los Ajustes. Verde en iOS, como el
 * del sistema; rojo de la app en el PC. */
export function Toggle({ checked, onChange, label, disabled = false }: { checked: boolean; onChange: (next: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative h-7 w-12 shrink-0 rounded-full outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/80 disabled:opacity-50 ${
        checked ? "bg-red-600 ios:bg-[#34c759]" : "bg-neutral-700"
      }`}
    >
      <span className={`absolute left-0 top-0.5 h-6 w-6 rounded-full bg-white shadow transition-transform duration-200 ${checked ? "translate-x-5" : "translate-x-0.5"}`} />
    </button>
  );
}
