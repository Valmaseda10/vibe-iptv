import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { BrandMark } from "../../components/BrandMark";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { WhatsNewSheet } from "../../components/WhatsNewSheet";
import { IS_IOS } from "../../lib/platform";
import { RELEASE_NOTES, useAppVersion } from "../../lib/releaseNotes";
import { BUTTON, BUTTON_PRIMARY } from "./styles";

type UpdateCheck = { state: "up_to_date" } | { state: "ready"; version: string; notes: string | null } | { state: "busy" } | { state: "failed"; message: string };

/** Ajustes → Acerca de: la versión, buscar actualizaciones a mano (PC), las novedades y el
 * registro de diagnóstico (PC). */
export function AboutSection() {
  const version = useAppVersion();
  const [whatsNew, setWhatsNew] = useState(false);
  const [diagnostics, setDiagnostics] = useState<{ ok: boolean; text: string } | null>(null);

  const check = useMutation({ mutationFn: () => invoke<UpdateCheck>("check_for_update") });
  const install = useMutation({ mutationFn: () => invoke<void>("install_update") });
  const exportLog = useMutation({
    mutationFn: () => invoke<string>("export_diagnostics_log"),
    onSuccess: (path) => setDiagnostics({ ok: true, text: `Guardado en ${path}` }),
    onError: (err) => setDiagnostics({ ok: false, text: String(err) }),
  });

  const result = check.data;
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center gap-3 rounded-2xl bg-gradient-to-b from-white/[0.07] to-transparent px-6 py-8 text-center ring-1 ring-white/[0.06] ios:rounded-[22px] ios:ring-0">
        <BrandMark className="h-16 w-16 drop-shadow-[0_8px_24px_rgba(255,45,85,0.35)]" />
        <div className="flex flex-col gap-0.5">
          <span className="text-2xl font-extrabold tracking-tight text-white">
            VIBE <span className="font-semibold text-white/55">IPTV</span>
          </span>
          <span className="text-sm text-neutral-400">Versión {version ?? "…"}</span>
          <span className="text-[12px] text-neutral-500">Compilación {__BUILD_ID__}</span>
        </div>
      </div>

      <SettingsCard title="Versión">
        {!IS_IOS && (
          <SettingsRow
            title="Buscar actualizaciones"
            description={
              !result
                ? "La app lo mira sola cada pocas horas; aquí puedes hacerlo ahora."
                : result.state === "up_to_date"
                  ? "Tienes la última versión."
                  : result.state === "ready"
                    ? `La versión ${result.version} está lista para instalar.`
                    : result.state === "busy"
                      ? "Hay algo reproduciéndose: se descargará al terminar."
                      : result.message
            }
          >
            {result?.state === "ready" ? (
              <button type="button" className={BUTTON_PRIMARY} onClick={() => install.mutate()} disabled={install.isPending}>
                {install.isPending ? "Instalando…" : "Reiniciar y actualizar"}
              </button>
            ) : (
              <button type="button" className={BUTTON} onClick={() => check.mutate()} disabled={check.isPending}>
                {check.isPending ? "Buscando…" : "Buscar ahora"}
              </button>
            )}
          </SettingsRow>
        )}
        {IS_IOS && <SettingsRow title="Actualizaciones" description="En iPhone y iPad la versión nueva se instala aparte, con el fichero .ipa." />}
        <SettingsRow title="Novedades" description={`Lo que trae la versión ${RELEASE_NOTES.version}.`}>
          <button type="button" className={BUTTON} onClick={() => setWhatsNew(true)}>
            Ver novedades
          </button>
        </SettingsRow>
      </SettingsCard>

      {!IS_IOS && (
        <SettingsCard title="Ayuda" footer="Nunca incluye usuario, contraseña ni direcciones del proveedor.">
          <SettingsRow title="Registro de diagnóstico" description="Guarda en un fichero las últimas líneas de registro de esta sesión, por si hay que resolver un problema.">
            <button type="button" className={BUTTON} onClick={() => exportLog.mutate()} disabled={exportLog.isPending}>
              {exportLog.isPending ? "Exportando…" : "Exportar registro"}
            </button>
          </SettingsRow>
          {diagnostics && <p className={`break-all px-4 py-3 text-[13px] ${diagnostics.ok ? "text-neutral-300" : "text-red-400"}`}>{diagnostics.text}</p>}
        </SettingsCard>
      )}

      {whatsNew && <WhatsNewSheet onClose={() => setWhatsNew(false)} />}
    </div>
  );
}
