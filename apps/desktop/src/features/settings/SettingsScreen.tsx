import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { apiGet, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type {
  AccountStatus,
  AccountStatusResponse,
  AddProviderInput,
  CloudSyncStatus,
  BinariesStatus,
  EpgSyncSummary,
  LanStatusResponse,
  M3uValidation,
  PairingStartResponse,
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
  const [epgSummary, setEpgSummary] = useState<EpgSyncSummary | null>(null);

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

  useEffect(() => {
    const unlistenPromise = listen<EpgSyncSummary>("epg://summary", (event) => setEpgSummary(event.payload));
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

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

  const deleteProviderMutation = useMutation({
    mutationFn: (providerId: number) => invoke<void>("delete_provider", { providerId }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["providers"] }),
  });

  function handleDeleteProvider(provider: ProviderSummary) {
    // Sin librería de diálogos todavía (F5 no la trajo): `confirm` nativo es tosco pero
    // esto es justo el tipo de acción — borra canales, películas y series enteros de
    // este proveedor — que no debería poder pasar por un clic accidental.
    if (window.confirm(`¿Eliminar "${provider.name}"? Se borrará todo su catálogo. No se puede deshacer.`)) {
      deleteProviderMutation.mutate(provider.id);
    }
  }

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

      <BinariesStatusBanner />

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

      <AccountSection />
      <ProfileSection />
      <LanSection />
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
                    <ExpiryWarning expiresAt={provider.expires_at} />
                  </div>
                  <div className="flex shrink-0 gap-2">
                    <button
                      type="button"
                      className="rounded bg-neutral-700 px-3 py-2 hover:bg-neutral-600"
                      onClick={() => syncMutation.mutate(provider.id)}
                      disabled={syncMutation.isPending}
                    >
                      Sincronizar
                    </button>
                    <button
                      type="button"
                      className="rounded bg-neutral-800 px-3 py-2 text-red-400 hover:bg-red-950 disabled:opacity-50"
                      onClick={() => handleDeleteProvider(provider)}
                      disabled={deleteProviderMutation.isPending}
                    >
                      Eliminar
                    </button>
                  </div>
                </div>
                {deleteProviderMutation.isError && deleteProviderMutation.variables === provider.id && (
                  <p className="mt-2 text-sm text-red-400">{String(deleteProviderMutation.error)}</p>
                )}
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
        {epgSummary && epgSummary.channels_without_epg > 0 && (
          <p className="mt-4 text-sm text-amber-400">
            {epgSummary.channels_without_epg} canal{epgSummary.channels_without_epg === 1 ? "" : "es"} sin guía de programación
            emparejada tras la última importación — el proveedor no dio un identificador que coincidiera con ninguno de
            la guía XMLTV.
          </p>
        )}
      </section>
    </div>
  );
}

/** El control parental global de F8 (un PIN para toda la app) se sustituyó por perfiles
 * reales: cada uno con su propio PIN de entrada y su propio "adulto"/"infantil" — se
 * administran desde el propio selector de perfiles (botón "Administrar perfiles"), no
 * desde Ajustes. Aquí solo queda un atajo para volver a ese selector. */
function ProfileSection() {
  const { setProfileId } = useActiveProfile();

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
      <h2 className="mb-4 text-xl font-medium">Perfil</h2>
      <p className="mb-3 text-sm text-neutral-400">
        Cambia de perfil, o crea/edita/borra perfiles y sus PIN desde el selector — cada perfil tiene su propia lista de
        favoritos, su "continuar viendo", y su propio PIN opcional.
      </p>
      <button type="button" className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={() => setProfileId(null)}>
        Cambiar de perfil
      </button>
    </section>
  );
}

/** Cuenta de sincronización: perfiles/favoritos/progreso viajan entre dispositivos vía
 * Firebase (excepción consciente a "el PC es el servidor" — ver docs/CLOUD-SYNC.md). El
 * catálogo, las credenciales de proveedor y el streaming en sí nunca pasan por aquí. */
function AccountSection() {
  const queryClient = useQueryClient();
  const { setProfileId } = useActiveProfile();
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: ["account-status"],
    queryFn: () => apiGet<AccountStatusResponse>("/api/account/status"),
    // Solo mientras Ajustes está abierto: para ver bajar los cambios pendientes.
    refetchInterval: 15_000,
  });

  function invalidate() {
    queryClient.invalidateQueries({ queryKey: ["account-status"] });
  }

  // Cambiar de cuenta (entrar o salir) puede significar que ahora manda otra persona
  // delante de la pantalla: se vuelve a preguntar el perfil en vez de arrastrar el que
  // quedó activo, igual que hace `AccountGate` justo después de iniciar sesión.
  function forgetActiveProfile() {
    setProfileId(null);
    queryClient.invalidateQueries({ queryKey: ["profiles"] });
  }

  const authMutation = useMutation({
    mutationFn: () => apiPost<AccountStatusResponse>(mode === "login" ? "/api/account/login" : "/api/account/signup", { email, password }),
    onSuccess: () => {
      setError(null);
      setPassword("");
      invalidate();
      forgetActiveProfile();
    },
    onError: (err) => setError(String(err).replace(/^Error:\s*/, "")),
  });

  const logoutMutation = useMutation({
    mutationFn: () => apiPost("/api/account/logout", {}),
    onSuccess: () => {
      // Invalidar "account-status" ya basta para que `AccountGate` (que envuelve toda
      // la app) vuelva a mostrar la pantalla de cuenta — no hace falta ninguna bandera
      // aparte de recordar/deshacer.
      invalidate();
      forgetActiveProfile();
    },
  });

  if (statusQuery.data?.logged_in) {
    return (
      <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
        <h2 className="mb-4 text-xl font-medium">Cuenta</h2>
        <p className="mb-3 text-sm text-neutral-300">Sesión iniciada como {statusQuery.data.email}.</p>
        {statusQuery.data.sync && <CloudSyncLine sync={statusQuery.data.sync} />}
        <p className="mb-4 text-xs text-neutral-500">
          Se sincronizan perfiles, favoritos y progreso — nunca las credenciales de tus proveedores IPTV, que se
          quedan solo en este PC.
        </p>
        <button
          type="button"
          className="mb-6 rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600"
          onClick={() => logoutMutation.mutate()}
          disabled={logoutMutation.isPending}
        >
          Cerrar sesión
        </button>
        <ChangePasswordForm />
      </section>
    );
  }

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
      <h2 className="mb-4 text-xl font-medium">Cuenta</h2>
      <p className="mb-4 text-sm text-neutral-400">
        Inicia sesión para que tus perfiles, favoritos y progreso te sigan a otros dispositivos donde uses VIBE IPTV. Sin
        cuenta, la app funciona igual, solo en este PC.
      </p>
      <div className="mb-4 flex gap-4" role="radiogroup" aria-label="Iniciar sesión o crear cuenta">
        {(["login", "signup"] as const).map((m) => (
          <label key={m} className="flex items-center gap-2">
            <input type="radio" checked={mode === m} onChange={() => setMode(m)} />
            {m === "login" ? "Iniciar sesión" : "Crear cuenta"}
          </label>
        ))}
      </div>
      <div className="mb-3 flex gap-3">
        <input
          type="email"
          placeholder="Correo"
          className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
        <input
          type="password"
          placeholder="Contraseña"
          className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <button
          type="button"
          className="shrink-0 rounded bg-red-600 px-4 py-2 hover:bg-red-500 disabled:opacity-50"
          onClick={() => authMutation.mutate()}
          disabled={!email || !password || authMutation.isPending}
        >
          {authMutation.isPending ? "…" : mode === "login" ? "Entrar" : "Crear cuenta"}
        </button>
      </div>
      {error && <p role="alert" className="text-red-400">{error}</p>}
    </section>
  );
}

/** Una línea de estado de la sincronización: si hay cambios esperando y por qué. Sin
 * conexión con la nube la app sigue funcionando igual; esto solo lo hace visible. */
function CloudSyncLine({ sync }: { sync: CloudSyncStatus }) {
  const pending = sync.pending_changes;
  const pendingText = `${pending} cambio${pending === 1 ? "" : "s"} esperando`;
  if (sync.last_error) {
    return (
      <p className="mb-3 text-sm text-amber-400">
        Sincronización en pausa: {sync.last_error}
        {pending > 0 && ` ${pendingText}; se enviarán solos cuando vuelva la conexión.`}
      </p>
    );
  }
  if (pending > 0) {
    return <p className="mb-3 text-sm text-neutral-400">Sincronizando… {pendingText}.</p>;
  }
  if (sync.last_synced_at) {
    return (
      <p className="mb-3 text-sm text-emerald-400">
        Todo sincronizado · {new Date(sync.last_synced_at * 1000).toLocaleTimeString()}
      </p>
    );
  }
  return <p className="mb-3 text-sm text-neutral-400">Conectando con la nube…</p>;
}

/** Cambiar contraseña con la sesión ya abierta (F "premium" — confianza de cuenta). El
 * backend vuelve a comprobar la actual contra la nube antes de fijar la nueva, aunque la
 * sesión ya esté abierta: no es cosa de esta pantalla, es la propia API la que lo exige. */
function ChangePasswordForm() {
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const mutation = useMutation({
    mutationFn: () => apiPost("/api/account/change-password", { current_password: currentPassword, new_password: newPassword }),
    onSuccess: () => {
      setError(null);
      setSuccess(true);
      setCurrentPassword("");
      setNewPassword("");
    },
    onError: (err) => {
      setSuccess(false);
      setError(String(err).replace(/^Error:\s*/, ""));
    },
  });

  return (
    <div className="border-t border-neutral-800 pt-4">
      <h3 className="mb-3 text-sm font-medium text-neutral-300">Cambiar contraseña</h3>
      <div className="flex flex-wrap gap-3">
        <input
          type="password"
          placeholder="Contraseña actual"
          className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm"
          value={currentPassword}
          onChange={(e) => setCurrentPassword(e.target.value)}
        />
        <input
          type="password"
          placeholder="Contraseña nueva (mínimo 6 caracteres)"
          className="flex-1 rounded border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm"
          value={newPassword}
          onChange={(e) => setNewPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && currentPassword && newPassword.length >= 6) mutation.mutate();
          }}
        />
        <button
          type="button"
          className="shrink-0 rounded bg-neutral-700 px-4 py-2 text-sm hover:bg-neutral-600 disabled:opacity-50"
          onClick={() => mutation.mutate()}
          disabled={!currentPassword || newPassword.length < 6 || mutation.isPending}
        >
          {mutation.isPending ? "…" : "Cambiar"}
        </button>
      </div>
      {error && <p role="alert" className="mt-2 text-sm text-red-400">{error}</p>}
      {success && <p className="mt-2 text-sm text-emerald-400">Contraseña actualizada.</p>}
    </div>
  );
}

/** Modo LAN (F10): abrir el servidor a la red local, emparejar dispositivos por código
 * de 6 dígitos, y revocar el acceso de un dispositivo. Sin ningún dispositivo real (el
 * cliente de Fire TV llega en una fase posterior) contra el que probar el emparejamiento
 * de punta a punta — ver `docs/DECISIONS.md`. */
function LanSection() {
  const queryClient = useQueryClient();
  const [pairingResult, setPairingResult] = useState<PairingStartResponse | null>(null);
  const [pairingError, setPairingError] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: ["lan-status"],
    queryFn: () => apiGet<LanStatusResponse>("/api/lan/status"),
    // El código de emparejamiento caduca solo (5 min); refrescar de vez en cuando basta
    // para que "visto por última vez" de la lista de dispositivos no se quede parado.
    refetchInterval: 15_000,
  });

  function invalidate() {
    queryClient.invalidateQueries({ queryKey: ["lan-status"] });
  }

  const enableMutation = useMutation({
    mutationFn: () => apiPost("/api/lan/enable", {}),
    onSuccess: invalidate,
  });
  const disableMutation = useMutation({
    mutationFn: () => apiPost("/api/lan/disable", {}),
    onSuccess: () => {
      setPairingResult(null);
      invalidate();
    },
  });
  const revokeMutation = useMutation({
    mutationFn: (deviceId: number) => apiPost(`/api/lan/devices/${deviceId}/revoke`, {}),
    onSuccess: invalidate,
  });

  async function startPairing() {
    setPairingError(null);
    try {
      const result = await apiPost<PairingStartResponse>("/api/lan/pairing/start", {});
      setPairingResult(result);
    } catch (error) {
      setPairingError(String(error));
    }
  }

  const enabled = statusQuery.data?.enabled ?? false;

  return (
    <section className="mb-10 rounded-lg border border-neutral-800 bg-neutral-900 p-6">
      <h2 className="mb-4 text-xl font-medium">Red local</h2>
      <p className="mb-4 text-sm text-neutral-400">
        Deja que otros dispositivos de tu red (por ejemplo, un futuro cliente de Fire TV) usen esta app como servidor.
        Desactivado, el servidor solo responde a este mismo PC — igual que hasta ahora.
      </p>

      <label className="mb-4 flex items-center gap-3">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => (e.target.checked ? enableMutation.mutate() : disableMutation.mutate())}
          disabled={enableMutation.isPending || disableMutation.isPending}
        />
        <span>Permitir acceso desde la red local</span>
      </label>

      {enabled && (
        <div className="rounded border border-neutral-700 p-4">
          <div className="mb-3 flex items-center gap-3">
            <button type="button" className="rounded bg-red-600 px-4 py-2 hover:bg-red-500" onClick={startPairing}>
              Emparejar dispositivo nuevo
            </button>
            {statusQuery.data?.local_address && (
              <span className="text-sm text-neutral-400">Esta PC: {statusQuery.data.local_address}</span>
            )}
          </div>
          {pairingError && <p className="mb-3 text-red-400">{pairingError}</p>}
          {pairingResult && (
            <div className="mb-4 rounded bg-neutral-800 p-4 text-center">
              <p className="mb-1 text-sm text-neutral-400">Introduce este código en el otro dispositivo:</p>
              <p className="font-mono text-4xl tracking-[0.3em]">{pairingResult.code}</p>
              <p className="mt-1 text-xs text-neutral-500">Caduca en {Math.round(pairingResult.expires_in_s / 60)} min</p>
            </div>
          )}

          <h3 className="mb-2 text-sm font-medium text-neutral-400">Dispositivos emparejados</h3>
          {statusQuery.data && statusQuery.data.devices.length === 0 ? (
            <p className="text-sm text-neutral-500">Ninguno todavía.</p>
          ) : (
            <ul className="flex flex-col gap-2">
              {statusQuery.data?.devices.map((device) => (
                <li key={device.id} className="flex items-center justify-between rounded bg-neutral-800 px-3 py-2 text-sm">
                  <div>
                    <span className={device.revoked ? "text-neutral-500 line-through" : ""}>{device.device_name}</span>
                    {device.last_seen_at && (
                      <span className="ml-2 text-xs text-neutral-500">
                        visto {new Date(device.last_seen_at * 1000).toLocaleString()}
                      </span>
                    )}
                  </div>
                  {!device.revoked && (
                    <button
                      type="button"
                      className="rounded bg-neutral-700 px-3 py-1 hover:bg-neutral-600"
                      onClick={() => revokeMutation.mutate(device.id)}
                    >
                      Revocar
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}

/** "Comprobación de que existen al arrancar" (F10): si falta ffmpeg, ffprobe o mpv —
 * algo que no debería pasar con el instalador (los empaqueta como sidecars), pero sí
 * puede pasar en desarrollo o si una instalación se corrompió — lo dice aquí en vez de
 * dejar que el usuario se entere solo al intentar reproducir algo. No se muestra nada
 * mientras todo está bien: es un aviso, no una sección permanente. */
const EXPIRY_WARNING_DAYS = 7;

/** Aviso de "Estado" (F "premium" — informe de I+D): el dato ya llegaba en
 * `ProviderSummary.expires_at` desde F2, pero no se mostraba en ningún sitio de forma
 * continua — solo un instante, al validar el proveedor antes de añadirlo. Una cuenta
 * IPTV que caduca sin avisar es justo el tipo de fallo mudo que conviene evitar. */
function ExpiryWarning({ expiresAt }: { expiresAt: number | null }) {
  if (expiresAt == null) return null;
  const daysLeft = Math.floor((expiresAt * 1000 - Date.now()) / (24 * 60 * 60 * 1000));
  if (daysLeft > EXPIRY_WARNING_DAYS) return null;

  const expired = daysLeft < 0;
  return (
    <p className={`mt-1 text-sm font-medium ${expired ? "text-red-400" : "text-amber-400"}`}>
      {expired
        ? `La cuenta del proveedor caducó el ${new Date(expiresAt * 1000).toLocaleDateString()}.`
        : `La cuenta del proveedor caduca en ${daysLeft} día${daysLeft === 1 ? "" : "s"} (${new Date(expiresAt * 1000).toLocaleDateString()}).`}
    </p>
  );
}

function BinariesStatusBanner() {
  const { data } = useQuery({
    queryKey: ["binaries-status"],
    queryFn: () => apiGet<BinariesStatus>("/api/playback/binaries"),
  });

  if (!data) return null;
  const missing = (["ffmpeg", "ffprobe", "mpv"] as const).filter((name) => !data[name]);
  if (missing.length === 0) return null;

  return (
    <div className="mb-6 rounded-lg border border-amber-700 bg-amber-950/40 p-4 text-sm text-amber-200">
      Falta{missing.length > 1 ? "n" : ""} <strong>{missing.join(", ")}</strong>: la reproducción de directo
      {missing.includes("mpv") ? " y algunos contenidos con códecs poco habituales" : ""} no va a funcionar hasta que
      estén disponibles. Si esto es una instalación desde el instalador, es un fallo real — reinstala la app.
    </div>
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
