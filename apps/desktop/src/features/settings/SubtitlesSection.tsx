import {
  SUBTITLE_COLORS,
  SUBTITLE_FONTS,
  SUBTITLE_LOOKS,
  SUBTITLE_SIZES,
  subtitlePreviewStyle,
  useSubtitleStyle,
  type SubtitleStyle,
} from "../../lib/subtitleStyle";

/** Ajustes → Subtítulos: tamaño, estilo, color y letra, por perfil, con vista previa. Se
 * aplica en el reproductor del PC y en el de iPhone y iPad. */
export function SubtitlesSection() {
  const [style, setStyle] = useSubtitleStyle();
  const set = (patch: Partial<SubtitleStyle>) => setStyle({ ...style, ...patch });

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-4 sm:p-6 ios:mb-8 ios:rounded-[22px] ios:border-0 ios:bg-ios-surface-1 ios:p-5">
      <h2 className="mb-4 text-xl font-medium ios:text-[22px] ios:font-bold ios:tracking-tight">Subtítulos</h2>

      <div
        className="mb-5 flex h-28 items-end justify-center rounded-lg bg-gradient-to-br from-sky-900 via-neutral-800 to-amber-900 p-3 ios:rounded-2xl"
        aria-label="Vista previa de los subtítulos"
      >
        <span className="text-center leading-snug" style={subtitlePreviewStyle(style)}>
          Así se verán los subtítulos
        </span>
      </div>

      <div className="flex flex-col gap-4">
        <Choice label="Tamaño" options={SUBTITLE_SIZES} value={style.size} onChange={(size) => set({ size })} />
        <Choice label="Estilo" options={SUBTITLE_LOOKS} value={style.look} onChange={(look) => set({ look })} />
        <Choice label="Color" options={SUBTITLE_COLORS} value={style.color} onChange={(color) => set({ color })} />
        <Choice label="Letra" options={SUBTITLE_FONTS} value={style.font} onChange={(font) => set({ font })} />
      </div>
    </section>
  );
}

function Choice({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-4">
      <span className="w-20 shrink-0 text-sm text-neutral-400 ios:text-[15px] ios:text-ios-label-2">{label}</span>
      <div className="flex flex-wrap gap-2">
        {options.map((option) => {
          const selected = option.value === value;
          return (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => onChange(option.value)}
              className={`rounded px-3 py-1.5 text-sm outline-none focus-visible:ring-2 focus-visible:ring-red-600 ios:rounded-full ios:px-4 ios:py-2 ios:text-[15px] ${
                selected ? "bg-red-600 text-white ios:bg-white ios:text-black" : "bg-neutral-800 text-neutral-300 hover:bg-neutral-700 ios:bg-white/10 ios:text-white"
              }`}
            >
              {option.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
