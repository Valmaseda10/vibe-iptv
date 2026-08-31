import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { apiGet, apiPost } from "../../lib/api-client";
import type {
  AccountStatus,
  AddProviderInput,
  M3uValidation,
  ParentalStatus,
  ProviderKind,
  ProviderSummary,
  SyncProgress,
} from "../../lib/api-types";

type FormState = {
  kind: ProviderKind;
  name: string;
  baseUrl: string;
  username: string;
  password: string;
  m3uUrl: string;
};

const emptyForm: FormState = { kind: "xtream", name: "", baseUrl: "", username: "", password: "", m3uUrl: "" };

/** Pantalla funcional de F2: alta de proveedor y validación en vivo. El diseño visual
 * definitivo llega en F5; aquí lo importante es que funcione contra un proveedor real. */
export function SettingsScreen() {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<FormState>(emptyForm);
  const [xtreamStatus, setXtreamStatus] = useState<AccountStatus | null>(null);
  const [m3uStatus, setM3uStatus] = useState<M3uValidation | null>(null);
  const [progressByProvider, setProgressByProvider] = useState<Record<number, SyncProgress>>({});
  const [tmdbProgress, setTmdbProgress] = useState<SyncProgress | null>(null);
  const [tmdbApiKey, setTmdbApiKey] = useState("");

  const providersQuery = useQuery({
    queryKey: ["providers"],
    queryFn: () => invoke<ProviderSummary[]>("list_providers"),
  });

  const tmdbKeyQuery = useQuery({
    queryKey: ["tmdb-api-key-configured"],
    queryFn: () => invoke<boolean>("has_tmdb_api_key"),
  });

  useEffect(() => {
    const unlistenPromise = listen<SyncProgress>("sync://progress", (event) => {
      // provider_id 0 es el pase global de TMDB (no está atado a un proveedor
      // concreto: películas y series ya no distinguen de qué proveedor vinieron).
      if (event.payload.stage === "tmdb") {
        setTmdbProgress(event.payload);
      } else {
        setProgressByProvider((prev) => ({ ...prev, [event.payload.provider_id]: event.payload }));
      }
      if (event.payload.total > 0 && event.payload.current >= event.payload.total) {
        queryClient.invalidateQueries({ queryKey: ["providers"] });
      }
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, [queryClient]);

  const validateMutation = useMutation({
    mutationFn: async () => {
      if (form.kind === "xtream") {
        const result = await invoke<AccountStatus>("validate_xtream", {
          baseUrl: form.baseUrl,
          username: form.username,
          password: form.password,
          userAgent: null,
        });
        return { kind: "xtream" as const, result };
      }
      const result = await invoke<M3uValidation>("validate_m3u", { url: form.m3uUrl });
      return { kind: "m3u" as const, result };
    },
    onSuccess: (data) => {
      if (data.kind === "xtream") {
        setXtreamStatus(data.result);
        setM3uStatus(null);
      } else {
        setM3uStatus(data.result);
        setXtreamStatus(null);
      }
    },
  });

  const addMutation = useMutation({
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
    onSuccess: () => {
      setForm(emptyForm);
      setXtreamStatus(null);
      setM3uStatus(null);
      queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
  });

  const syncMutation = useMutation({
    mutationFn: (providerId: number) => invoke<void>("sync_provider", { providerId }),
  });

  const saveTmdbKeyMutation = useMutation({
    mutationFn: (apiKey: string) => invoke<void>("set_tmdb_api_key", { apiKey }),
    onSuccess: () => {
      setTmdbApiKey("");
      queryClient.invalidateQueries({ queryKey: ["tmdb-api-key-configured"] });
    },
  });

  const matchTmdbMutation = useMutation({
    mutationFn: () => invoke<void>("match_tmdb"),
  });

  function applySuggestion() {
    const suggestion = m3uStatus?.suggested_xtream;
    if (!suggestion) return;
    setForm((prev) => ({
      ...prev,
      kind: "xtream",
      baseUrl: suggestion.base_url,
      username: suggestion.username,
      password: suggestion.password,
    }));
    setM3uStatus(null);
  }

  return (
    <div className="mx-auto max-w-3xl p-8">
      <h1 className="mb-6 text-3xl font-semibold">Ajustes — Proveedores</h1>

      <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
        <h2 className="mb-4 text-xl font-medium">Añadir proveedor</h2>

        <div className="mb-4 flex gap-4" role="radiogroup" aria-label="Tipo de proveedor">
          {(["xtream", "m3u"] as const).map((kind) => (
            <label key={kind} className="flex items-center gap-2">
              <input
                type="radio"
                name="kind"
                value={kind}
                checked={form.kind === kind}
                onChange={() => setForm((prev) => ({ ...prev, kind }))}
              />
              {kind === "xtream" ? "Xtream Codes" : "Lista M3U"}
            </label>
          ))}
        </div>

        <label className="mb-3 block">
          <span className="mb-1 block text-sm text-neutral-400">Nombre</span>
          <input
            className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={form.name}
            onChange={(e) => setForm((prev) => ({ ...prev, name: e.target.value }))}
          />
        </label>

        {form.kind === "xtream" ? (
          <>
            <label className="mb-3 block">
              <span className="mb-1 block text-sm text-neutral-400">URL del panel (http://host:puerto)</span>
              <input
                className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
                value={form.baseUrl}
                onChange={(e) => setForm((prev) => ({ ...prev, baseUrl: e.target.value }))}
              />
            </label>
            <div className="mb-3 flex gap-3">
              <label className="flex-1">
                <span className="mb-1 block text-sm text-neutral-400">Usuario</span>
                <input
                  className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
                  value={form.username}
                  onChange={(e) => setForm((prev) => ({ ...prev, username: e.target.value }))}
                />
              </label>
              <label className="flex-1">
                <span className="mb-1 block text-sm text-neutral-400">Contraseña</span>
                <input
                  type="password"
                  className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
                  value={form.password}
                  onChange={(e) => setForm((prev) => ({ ...prev, password: e.target.value }))}
                />
              </label>
            </div>
          </>
        ) : (
          <label className="mb-3 block">
            <span className="mb-1 block text-sm text-neutral-400">URL de la lista M3U</span>
            <input
              className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
              value={form.m3uUrl}
              onChange={(e) => setForm((prev) => ({ ...prev, m3uUrl: e.target.value }))}
            />
          </label>
        )}

        <div className="flex gap-3">
          <button
            type="button"
            className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600"
            onClick={() => validateMutation.mutate()}
            disabled={validateMutation.isPending}
          >
            {validateMutation.isPending ? "Comprobando…" : "Validar"}
          </button>
          <button
            type="button"
            className="rounded bg-red-600 px-4 py-2 hover:bg-red-500 disabled:opacity-50"
            onClick={() => addMutation.mutate()}
            disabled={addMutation.isPending || !form.name}
          >
            {addMutation.isPending ? "Añadiendo…" : "Añadir y sincronizar"}
          </button>
        </div>

        {validateMutation.isError && <p className="mt-3 text-red-400">{String(validateMutation.error)}</p>}
        {addMutation.isError && <p className="mt-3 text-red-400">{String(addMutation.error)}</p>}

        {xtreamStatus && (
          <div className="mt-4 rounded border border-neutral-700 p-4">
            <p>
              Estado: {xtreamStatus.auth_ok ? "válido" : "credenciales rechazadas"} ({xtreamStatus.status})
            </p>
            <p>Conexiones máximas: {xtreamStatus.max_connections}</p>
            {xtreamStatus.expires_at && <p>Caduca: {new Date(xtreamStatus.expires_at * 1000).toLocaleDateString()}</p>}
          </div>
        )}

        {m3uStatus && (
          <div className="mt-4 rounded border border-neutral-700 p-4">
            <p>
              {m3uStatus.channel_count} canales, {m3uStatus.movie_count} películas, {m3uStatus.series_count} series
            </p>
            {m3uStatus.suggested_xtream && (
              <div className="mt-2 flex items-center gap-3 rounded bg-neutral-800 p-3">
                <span>Esta URL es en realidad un proveedor Xtream.</span>
                <button type="button" className="rounded bg-red-600 px-3 py-1 hover:bg-red-500" onClick={applySuggestion}>
                  Usar como Xtream
                </button>
              </div>
            )}
          </div>
        )}
      </section>

      <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
        <h2 className="mb-4 text-xl font-medium">TMDB</h2>
        <p className="mb-3 text-sm text-neutral-400">
          {tmdbKeyQuery.data ? "Clave de API guardada." : "Sin clave de API guardada todavía. Sin ella, las películas y series se quedan sin portada ni sinopsis."}
        </p>
        <div className="flex gap-3">
          <input
            type="password"
            placeholder="Clave de API de TMDB"
            className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={tmdbApiKey}
            onChange={(e) => setTmdbApiKey(e.target.value)}
          />
          <button
            type="button"
            className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600 disabled:opacity-50"
            onClick={() => saveTmdbKeyMutation.mutate(tmdbApiKey)}
            disabled={!tmdbApiKey || saveTmdbKeyMutation.isPending}
          >
            {saveTmdbKeyMutation.isPending ? "Guardando…" : "Guardar"}
          </button>
          <button
            type="button"
            className="rounded bg-red-600 px-4 py-2 hover:bg-red-500 disabled:opacity-50"
            onClick={() => matchTmdbMutation.mutate()}
            disabled={!tmdbKeyQuery.data || matchTmdbMutation.isPending}
          >
            Buscar en TMDB
          </button>
        </div>
        {matchTmdbMutation.isError && <p className="mt-3 text-red-400">{String(matchTmdbMutation.error)}</p>}
        {tmdbProgress && tmdbProgress.total > 0 && (
          <div className="mt-4">
            <p className="mb-1 text-sm text-neutral-400">
              Emparejando con TMDB: {tmdbProgress.current}/{tmdbProgress.total}
            </p>
            <div className="h-2 w-full overflow-hidden rounded bg-neutral-800">
              <div
                className="h-full bg-red-600 transition-[width]"
                style={{ width: `${Math.min(100, (tmdbProgress.current / tmdbProgress.total) * 100)}%` }}
              />
            </div>
          </div>
        )}
      </section>

      <ParentalControlSection />
      <DiagnosticsSection />

      <section>
        <h2 className="mb-4 text-xl font-medium">Proveedores</h2>
        {providersQuery.isPending && <p className="text-neutral-400">Cargando…</p>}
        {providersQuery.data?.length === 0 && <p className="text-neutral-400">Todavía no has añadido ninguno.</p>}
        <ul className="flex flex-col gap-3">
          {providersQuery.data?.map((provider) => {
            const progress = progressByProvider[provider.id];
            return (
              <li key={provider.id} className="rounded border border-neutral-800 p-4">
                <div className="flex items-center justify-between gap-4">
                  <div>
                    <p className="font-medium">{provider.name}</p>
                    <p className="text-sm text-neutral-400">
                      {provider.kind === "xtream" ? "Xtream" : "M3U"} · {provider.base_url_display}
                      {provider.username ? ` · ${provider.username}` : ""}
                    </p>
                    <p className="text-sm text-neutral-400">
                      Conexiones máx.: {provider.max_connections}
                      {provider.last_sync_at
                        ? ` · última sincronización: ${new Date(provider.last_sync_at * 1000).toLocaleString()}`
                        : " · sin sincronizar"}
                    </p>
                  </div>
                  <button
                    type="button"
                    className="shrink-0 rounded bg-neutral-700 px-3 py-2 hover:bg-neutral-600"
                    onClick={() => syncMutation.mutate(provider.id)}
                    disabled={syncMutation.isPending}
                  >
                    Sincronizar
                  </button>
                </div>
                {progress && progress.total > 0 && (
                  <div className="mt-3">
                    <p className="mb-1 text-sm text-neutral-400">
                      {progress.stage}: {progress.current}/{progress.total}
                    </p>
                    <div className="h-2 w-full overflow-hidden rounded bg-neutral-800">
                      <div
                        className="h-full bg-red-600 transition-[width]"
                        style={{ width: `${Math.min(100, (progress.current / progress.total) * 100)}%` }}
                      />
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </section>
    </div>
  );
}

/** Control parental (F8): un PIN oculta las categorías de canales y películas marcadas
 * como adultas (`is_adult`) hasta que se desbloquea. El desbloqueo nunca se guarda: se
 * pierde al reiniciar la app (ver `util::parental` en el núcleo). */
function ParentalControlSection() {
  const queryClient = useQueryClient();
  const [pinInput, setPinInput] = useState("");
  const [newPinInput, setNewPinInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: ["parental-status"],
    queryFn: () => apiGet<ParentalStatus>("/api/parental/status"),
  });

  function invalidate() {
    queryClient.invalidateQueries({ queryKey: ["parental-status"] });
  }

  const unlockMutation = useMutation({
    mutationFn: (pin: string) => apiPost("/api/parental/unlock", { pin }),
    onSuccess: () => {
      setPinInput("");
      setError(null);
      invalidate();
    },
    onError: () => setError("PIN incorrecto."),
  });

  const lockMutation = useMutation({
    mutationFn: () => apiPost("/api/parental/lock", {}),
    onSuccess: invalidate,
  });

  const setPinMutation = useMutation({
    mutationFn: (pin: string | null) => apiPost("/api/parental/pin", { pin }),
    onSuccess: () => {
      setNewPinInput("");
      setError(null);
      invalidate();
    },
    onError: () => setError("No se pudo cambiar el PIN."),
  });

  const status = statusQuery.data;

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
      <h2 className="mb-4 text-xl font-medium">Control parental</h2>
      <p className="mb-3 text-sm text-neutral-400">
        Con un PIN configurado, las categorías de canales y películas marcadas como para adultos se ocultan hasta que lo
        introduzcas. El desbloqueo dura solo mientras la app esté abierta.
      </p>

      {error && <p className="mb-3 text-red-400">{error}</p>}

      {!status ? null : !status.has_pin ? (
        <div className="flex gap-3">
          <input
            type="password"
            inputMode="numeric"
            placeholder="Nuevo PIN"
            className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={newPinInput}
            onChange={(e) => setNewPinInput(e.target.value)}
          />
          <button
            type="button"
            className="rounded bg-red-600 px-4 py-2 hover:bg-red-500 disabled:opacity-50"
            onClick={() => setPinMutation.mutate(newPinInput)}
            disabled={!newPinInput || setPinMutation.isPending}
          >
            Establecer PIN
          </button>
        </div>
      ) : !status.unlocked ? (
        <div className="flex gap-3">
          <input
            type="password"
            inputMode="numeric"
            placeholder="PIN"
            className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={pinInput}
            onChange={(e) => setPinInput(e.target.value)}
          />
          <button
            type="button"
            className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600 disabled:opacity-50"
            onClick={() => unlockMutation.mutate(pinInput)}
            disabled={!pinInput || unlockMutation.isPending}
          >
            Desbloquear
          </button>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            <span className="rounded bg-neutral-800 px-3 py-1 text-sm text-neutral-300">Desbloqueado por esta sesión</span>
            <button type="button" className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={() => lockMutation.mutate()}>
              Bloquear ahora
            </button>
          </div>
          <div className="flex gap-3">
            <input
              type="password"
              inputMode="numeric"
              placeholder="PIN nuevo (vacío para quitarlo)"
              className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
              value={newPinInput}
              onChange={(e) => setNewPinInput(e.target.value)}
            />
            <button
              type="button"
              className="rounded bg-red-600 px-4 py-2 hover:bg-red-500"
              onClick={() => setPinMutation.mutate(newPinInput || null)}
            >
              {newPinInput ? "Cambiar PIN" : "Quitar PIN"}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

/** Registro de diagnóstico exportable (F9): vuelca a un fichero las últimas líneas de
 * log capturadas en memoria. Ya vienen sin credenciales (ver `util::diag_log` en el
 * núcleo: toda URL de proveedor pasa por `redact()` antes de llegar a un log). */
function DiagnosticsSection() {
  const [result, setResult] = useState<{ ok: true; path: string } | { ok: false; error: string } | null>(null);
  const [exporting, setExporting] = useState(false);

  async function handleExport() {
    setExporting(true);
    setResult(null);
    try {
      const path = await invoke<string>("export_diagnostics_log");
      setResult({ ok: true, path });
    } catch (error) {
      setResult({ ok: false, error: String(error) });
    } finally {
      setExporting(false);
    }
  }

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
      <h2 className="mb-4 text-xl font-medium">Diagnóstico</h2>
      <p className="mb-3 text-sm text-neutral-400">
        Guarda en un fichero las últimas líneas de registro de esta sesión, por si necesitas compartirlas para resolver un
        problema. Nunca incluye usuario, contraseña ni URLs del proveedor.
      </p>
      <button
        type="button"
        className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600 disabled:opacity-50"
        onClick={handleExport}
        disabled={exporting}
      >
        {exporting ? "Exportando…" : "Exportar registro de diagnóstico"}
      </button>
      {result?.ok && <p className="mt-3 text-sm text-neutral-300">Guardado en: {result.path}</p>}
      {result && !result.ok && <p className="mt-3 text-red-400">{result.error}</p>}
    </section>
  );
}
