import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../../lib/api-client";
import type { AccountStatus, AddProviderInput, M3uValidation, ProviderKind, ProviderSummary } from "../../lib/api-types";
import { GlassSheet } from "../../components/GlassSheet";
import { BUTTON, BUTTON_PRIMARY, INPUT } from "./styles";

type FormState = { kind: ProviderKind; name: string; baseUrl: string; username: string; password: string; m3uUrl: string };
const EMPTY: FormState = { kind: "xtream", name: "", baseUrl: "", username: "", password: "", m3uUrl: "" };

const KINDS: { kind: ProviderKind; title: string; text: string }[] = [
  { kind: "xtream", title: "Xtream Codes", text: "Dirección del panel, usuario y contraseña. Lo más rápido y completo." },
  { kind: "m3u", title: "Lista M3U", text: "Un enlace a una lista .m3u o .m3u8." },
];

/** «Añadir proveedor» (2026-11-02): tipo, datos, comprobar y añadir, en una hoja que solo
 * sale al pedirla. Comprobar habla una vez con el proveedor para ver si la cuenta vale; no
 * abre ningún vídeo (R1). */
export function AddProviderSheet({ onClose, onAdded }: { onClose: () => void; onAdded: (provider: ProviderSummary) => void }) {
  const [form, setForm] = useState<FormState>(EMPTY);
  const [xtreamStatus, setXtreamStatus] = useState<AccountStatus | null>(null);
  const [m3uStatus, setM3uStatus] = useState<M3uValidation | null>(null);
  const set = (patch: Partial<FormState>) => setForm((prev) => ({ ...prev, ...patch }));

  const validate = useMutation({
    mutationFn: async () => {
      if (form.kind === "xtream") {
        const result = await invoke<AccountStatus>("validate_xtream", { baseUrl: form.baseUrl, username: form.username, password: form.password, userAgent: null });
        return { kind: "xtream" as const, result };
      }
      return { kind: "m3u" as const, result: await invoke<M3uValidation>("validate_m3u", { url: form.m3uUrl }) };
    },
    onSuccess: (data) => {
      setXtreamStatus(data.kind === "xtream" ? data.result : null);
      setM3uStatus(data.kind === "m3u" ? data.result : null);
    },
  });

  const add = useMutation({
    mutationFn: () => {
      const input: AddProviderInput = {
        kind: form.kind,
        name: form.name,
        base_url: form.kind === "xtream" ? form.baseUrl : "",
        username: form.kind === "xtream" ? form.username : null,
        password: form.kind === "xtream" ? form.password : null,
        m3u_url: form.kind === "m3u" ? form.m3uUrl : null,
        user_agent: null,
      };
      return invoke<ProviderSummary>("add_provider", { input });
    },
    onSuccess: onAdded,
  });

  // Una lista M3U que en realidad sale de un panel Xtream: mejor como Xtream.
  function applySuggestion() {
    const suggestion = m3uStatus?.suggested_xtream;
    if (!suggestion) return;
    set({ kind: "xtream", baseUrl: suggestion.base_url, username: suggestion.username, password: suggestion.password });
    setM3uStatus(null);
  }

  const complete = form.name.trim() !== "" && (form.kind === "xtream" ? form.baseUrl && form.username && form.password : form.m3uUrl);

  return (
    <GlassSheet title="Añadir proveedor" onClose={onClose}>
      <div className="flex max-h-[80vh] flex-col gap-4 overflow-y-auto px-3 pb-3">
        <div role="radiogroup" aria-label="Tipo de proveedor" className="grid grid-cols-2 gap-2">
          {KINDS.map(({ kind, title, text }) => {
            const selected = form.kind === kind;
            return (
              <button
                key={kind}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => set({ kind })}
                className={`flex flex-col gap-1 rounded-xl p-3 text-left outline-none ring-1 transition-colors focus-visible:ring-2 focus-visible:ring-white/80 ${
                  selected ? "bg-red-600/15 ring-red-500" : "bg-white/[0.04] ring-white/10 hover:bg-white/[0.07]"
                }`}
              >
                <span className="text-sm font-semibold text-white">{title}</span>
                <span className="text-[12px] leading-snug text-neutral-400">{text}</span>
              </button>
            );
          })}
        </div>

        <Field label="Nombre">
          <input className={INPUT} value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="Por ejemplo, «Mi lista»" autoFocus />
        </Field>
        {form.kind === "xtream" ? (
          <>
            <Field label="Dirección del panel">
              <input className={INPUT} value={form.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="http://servidor:puerto" inputMode="url" autoCapitalize="off" />
            </Field>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Field label="Usuario">
                <input className={INPUT} value={form.username} onChange={(e) => set({ username: e.target.value })} autoCapitalize="off" autoComplete="off" />
              </Field>
              <Field label="Contraseña">
                <input type="password" className={INPUT} value={form.password} onChange={(e) => set({ password: e.target.value })} autoComplete="off" />
              </Field>
            </div>
          </>
        ) : (
          <Field label="Enlace de la lista M3U">
            <input className={INPUT} value={form.m3uUrl} onChange={(e) => set({ m3uUrl: e.target.value })} inputMode="url" autoCapitalize="off" />
          </Field>
        )}

        {xtreamStatus && (
          <div className={`rounded-xl p-3 text-sm ${xtreamStatus.auth_ok ? "bg-emerald-500/10 text-emerald-300" : "bg-red-500/10 text-red-300"}`}>
            <p className="font-medium">{xtreamStatus.auth_ok ? "La cuenta funciona." : `El proveedor rechazó los datos (${xtreamStatus.status}).`}</p>
            {xtreamStatus.auth_ok && (
              <p className="text-neutral-300">
                {xtreamStatus.max_connections} conexión{xtreamStatus.max_connections === 1 ? "" : "es"} a la vez
                {xtreamStatus.expires_at ? ` · caduca el ${new Date(xtreamStatus.expires_at * 1000).toLocaleDateString()}` : ""}
              </p>
            )}
          </div>
        )}
        {m3uStatus && (
          <div className="flex flex-col gap-2 rounded-xl bg-white/[0.05] p-3 text-sm text-neutral-200">
            <p>
              {m3uStatus.channel_count} canales, {m3uStatus.movie_count} películas, {m3uStatus.series_count} series
            </p>
            {m3uStatus.suggested_xtream && (
              <div className="flex flex-col gap-2 rounded-lg bg-red-600/10 p-3 ring-1 ring-red-600/40">
                <p className="font-medium text-white">Mejor como Xtream: esta lista viene de un panel Xtream.</p>
                <p className="text-neutral-300">Así el catálogo carga en segundos, con guía, series completas y lo ya emitido de los canales que lo tengan.</p>
                <button type="button" className={`${BUTTON_PRIMARY} self-start`} onClick={applySuggestion}>
                  Usar como Xtream
                </button>
              </div>
            )}
          </div>
        )}
        {validate.isError && <p className="text-sm text-red-400">{errorMessage(validate.error)}</p>}
        {add.isError && <p className="text-sm text-red-400">{errorMessage(add.error)}</p>}

        <div className="flex flex-wrap justify-end gap-2 pt-1">
          <button type="button" className={BUTTON} onClick={onClose}>
            Cancelar
          </button>
          <button type="button" className={BUTTON} onClick={() => validate.mutate()} disabled={!complete || validate.isPending}>
            {validate.isPending ? "Comprobando…" : "Comprobar"}
          </button>
          <button type="button" className={BUTTON_PRIMARY} onClick={() => add.mutate()} disabled={!complete || add.isPending}>
            {add.isPending ? "Añadiendo…" : "Añadir y sincronizar"}
          </button>
        </div>
      </div>
    </GlassSheet>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-[13px] font-medium text-neutral-400">{label}</span>
      {children}
    </label>
  );
}
