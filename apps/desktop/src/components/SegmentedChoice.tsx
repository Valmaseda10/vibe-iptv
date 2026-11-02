/** Elegir una opción entre pocas (tamaño de subtítulos, segundos de la cuenta atrás...):
 * botones juntos en una cápsula. Flechas izquierda/derecha para moverse entre ellos. */
export function SegmentedChoice<T extends string | number>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  function onKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const index = options.findIndex((o) => o.value === value);
    const next = options[index + (e.key === "ArrowRight" ? 1 : -1)];
    if (!next) return;
    e.preventDefault();
    onChange(next.value);
    const buttons = e.currentTarget.querySelectorAll<HTMLButtonElement>("button");
    buttons[options.indexOf(next)]?.focus();
  }

  return (
    <div role="radiogroup" aria-label={label} onKeyDown={onKeyDown} className="flex flex-wrap gap-1 rounded-xl bg-black/30 p-1 ios:rounded-full ios:bg-white/[0.08]">
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option.value)}
            className={`rounded-lg px-3 py-1.5 text-[13px] font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white/80 ios:rounded-full ios:px-3.5 ${
              selected ? "bg-white text-black shadow" : "text-neutral-300 hover:bg-white/10 hover:text-white"
            }`}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
