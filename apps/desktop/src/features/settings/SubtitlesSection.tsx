import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { SegmentedChoice } from "../../components/SegmentedChoice";
import {
  SUBTITLE_COLORS,
  SUBTITLE_FONTS,
  SUBTITLE_LOOKS,
  SUBTITLE_SIZES,
  subtitlePreviewStyle,
  useSubtitleStyle,
  type SubtitleStyle,
} from "../../lib/subtitleStyle";

/** Ajustes → Reproducción → Subtítulos: tamaño, estilo, color y letra, por perfil, con
 * vista previa. Se aplica en el reproductor del PC y en el de iPhone y iPad. */
export function SubtitlesSection() {
  const [style, setStyle] = useSubtitleStyle();
  const set = (patch: Partial<SubtitleStyle>) => setStyle({ ...style, ...patch });

  return (
    <SettingsCard title="Subtítulos">
      <div className="p-3">
        <div
          className="flex h-32 items-end justify-center rounded-lg bg-[radial-gradient(ellipse_at_30%_20%,#1e3a5f,transparent_60%),radial-gradient(ellipse_at_80%_80%,#6b3a12,transparent_55%),#18181b] p-4 ios:rounded-2xl"
          aria-label="Vista previa de los subtítulos"
        >
          <span className="text-center leading-snug" style={subtitlePreviewStyle(style)}>
            Así se verán los subtítulos
          </span>
        </div>
      </div>
      <SettingsRow title="Tamaño">
        <SegmentedChoice label="Tamaño" options={SUBTITLE_SIZES} value={style.size} onChange={(size) => set({ size })} />
      </SettingsRow>
      <SettingsRow title="Estilo">
        <SegmentedChoice label="Estilo" options={SUBTITLE_LOOKS} value={style.look} onChange={(look) => set({ look })} />
      </SettingsRow>
      <SettingsRow title="Color">
        <SegmentedChoice label="Color" options={SUBTITLE_COLORS} value={style.color} onChange={(color) => set({ color })} />
      </SettingsRow>
      <SettingsRow title="Letra">
        <SegmentedChoice label="Letra" options={SUBTITLE_FONTS} value={style.font} onChange={(font) => set({ font })} />
      </SettingsRow>
    </SettingsCard>
  );
}
