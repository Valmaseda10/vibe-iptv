import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { SegmentedChoice } from "../../components/SegmentedChoice";
import { Toggle } from "../../components/Toggle";
import { AUDIO_LANGUAGE_OPTIONS, NEXT_COUNTDOWN_OPTIONS, RESUME_REWIND_OPTIONS, usePlaybackPrefs, type PlaybackPrefs } from "../../lib/playbackPrefs";
import { IS_IOS } from "../../lib/platform";
import { SubtitlesSection } from "./SubtitlesSection";

/** Ajustes → Reproducción: siguiente episodio, reanudar, idioma de audio y el aspecto de
 * los subtítulos. Por perfil y en cada aparato. */
export function PlaybackSection() {
  const [prefs, setPrefs] = usePlaybackPrefs();
  const set = (patch: Partial<PlaybackPrefs>) => setPrefs({ ...prefs, ...patch });

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="Series y sagas">
        <SettingsRow inline title="Siguiente episodio solo" description="Al llegar a los créditos pasa al siguiente episodio (o a la siguiente película de la saga). Apagado, se ofrece pero esperas tú.">
          <Toggle checked={prefs.autoplayNext} onChange={(autoplayNext) => set({ autoplayNext })} label="Siguiente episodio solo" />
        </SettingsRow>
        {prefs.autoplayNext && (
          <SettingsRow title="Cuenta atrás" description="Cuánto esperar antes de pasar al siguiente.">
            <SegmentedChoice label="Cuenta atrás" options={NEXT_COUNTDOWN_OPTIONS} value={prefs.nextCountdown} onChange={(nextCountdown) => set({ nextCountdown })} />
          </SettingsRow>
        )}
      </SettingsCard>

      <SettingsCard title="Seguir viendo">
        <SettingsRow title="Retroceder al reanudar" description="Empieza unos segundos antes de donde lo dejaste, para situarte.">
          <SegmentedChoice label="Retroceder al reanudar" options={RESUME_REWIND_OPTIONS} value={prefs.resumeRewind} onChange={(resumeRewind) => set({ resumeRewind })} />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Audio"
        footer={IS_IOS ? undefined : "En el PC, cambiar de pista vuelve a abrir el vídeo un instante en el mismo punto."}
      >
        <SettingsRow title="Idioma de audio preferido" description="Si la película o el episodio lo trae, empieza en ese idioma.">
          <SegmentedChoice label="Idioma de audio preferido" options={AUDIO_LANGUAGE_OPTIONS} value={prefs.audioLanguage} onChange={(audioLanguage) => set({ audioLanguage })} />
        </SettingsRow>
      </SettingsCard>

      <SubtitlesSection />
    </div>
  );
}
