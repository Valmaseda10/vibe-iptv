import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, errorMessage } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { AccountStatusResponse, CloudSyncStatus } from "../../lib/api-types";
import { ProfileAvatar, useActiveProfileSummary } from "../../components/ProfileAvatar";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { SegmentedChoice } from "../../components/SegmentedChoice";
import { ChangePasswordForm } from "./ChangePasswordForm";
import { BUTTON, BUTTON_PRIMARY, INPUT } from "./styles";

/** Ajustes → Cuenta y perfiles. La cuenta sincroniza perfiles, Mi lista y progreso entre
 * aparatos vía Firebase (excepción consciente a «el PC es el servidor», ver
 * docs/CLOUD-SYNC.md); el catálogo, las credenciales de proveedor y el vídeo nunca pasan
 * por ella. */
export function AccountSection() {
  const queryClient = useQueryClient();
  const { setProfileId } = useActiveProfile();
  const profile = useActiveProfileSummary();
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

  // Cambiar de cuenta (entrar o salir) puede significar que ahora manda otra persona
  // delante de la pantalla: se vuelve a preguntar el perfil, como hace `AccountGate`.
  function forgetActiveProfile() {
    setProfileId(null);
    queryClient.invalidateQueries({ queryKey: ["profiles"] });
  }

  const authMutation = useMutation({
    mutationFn: () => apiPost<AccountStatusResponse>(mode === "login" ? "/api/account/login" : "/api/account/signup", { email, password }),
    onSuccess: () => {
      setError(null);
      setPassword("");
      queryClient.invalidateQueries({ queryKey: ["account-status"] });
      forgetActiveProfile();
    },
    onError: (err) => setError(errorMessage(err)),
  });

  const logoutMutation = useMutation({
    mutationFn: () => apiPost("/api/account/logout", {}),
    onSuccess: () => {
      // Con «account-status» invalidado, `AccountGate` vuelve a enseñar la pantalla de cuenta.
      queryClient.invalidateQueries({ queryKey: ["account-status"] });
      forgetActiveProfile();
    },
  });

  const status = statusQuery.data;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center gap-4 rounded-2xl bg-gradient-to-br from-white/[0.08] to-white/[0.02] p-5 ring-1 ring-white/[0.06] ios:rounded-[22px] ios:ring-0">
        <ProfileAvatar className="h-16 w-16 text-3xl" />
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-xl font-bold text-white">{profile?.name ?? ""}</span>
          <span className="truncate text-sm text-neutral-400">{status?.logged_in ? status.email : "Sin cuenta: todo se queda en este aparato"}</span>
        </div>
        <button type="button" className={BUTTON} onClick={() => setProfileId(null)}>
          Cambiar de perfil
        </button>
      </div>

      <SettingsCard title="Perfiles" footer="Cada perfil tiene su Mi lista, su «Seguir viendo», sus gustos y su PIN opcional. Se crean, editan y borran desde el selector de perfiles.">
        <SettingsRow title="Administrar perfiles" description="Nombre, avatar, PIN y si es infantil.">
          <button type="button" className={BUTTON} onClick={() => setProfileId(null)}>
            Abrir el selector
          </button>
        </SettingsRow>
      </SettingsCard>

      {status?.logged_in ? (
        <>
          <SettingsCard
            title="Cuenta en la nube"
            footer="Se sincronizan perfiles, Mi lista, progreso, gustos, equipos y recordatorios. Nunca los datos de tus proveedores IPTV, que se quedan en cada aparato."
          >
            <SettingsRow title="Sesión iniciada" description={status.email}>
              <button type="button" className={BUTTON} onClick={() => logoutMutation.mutate()} disabled={logoutMutation.isPending}>
                Cerrar sesión
              </button>
            </SettingsRow>
            {status.sync && <SettingsRow title="Sincronización" description={<CloudSyncText sync={status.sync} />} />}
          </SettingsCard>
          <SettingsCard title="Contraseña">
            <ChangePasswordForm />
          </SettingsCard>
        </>
      ) : (
        <SettingsCard title="Cuenta en la nube" footer="Sin cuenta la app funciona igual, solo en este aparato.">
          <SettingsRow title="Lleva tus perfiles a todos tus aparatos" description="Perfiles, Mi lista y progreso te siguen al iPhone, al iPad o a otro PC." stacked>
            <div className="flex w-full flex-col gap-3">
              <SegmentedChoice
                label="Iniciar sesión o crear cuenta"
                options={[
                  { value: "login", label: "Iniciar sesión" },
                  { value: "signup", label: "Crear cuenta" },
                ]}
                value={mode}
                onChange={setMode}
              />
              <div className="flex flex-col gap-3 sm:flex-row">
                <input type="email" placeholder="Correo" aria-label="Correo" className={INPUT} value={email} onChange={(e) => setEmail(e.target.value)} />
                <input
                  type="password"
                  placeholder="Contraseña"
                  aria-label="Contraseña"
                  className={INPUT}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && email && password) authMutation.mutate();
                  }}
                />
                <button type="button" className={BUTTON_PRIMARY} onClick={() => authMutation.mutate()} disabled={!email || !password || authMutation.isPending}>
                  {authMutation.isPending ? "…" : mode === "login" ? "Entrar" : "Crear cuenta"}
                </button>
              </div>
              {error && (
                <p role="alert" className="text-sm text-red-400">
                  {error}
                </p>
              )}
            </div>
          </SettingsRow>
        </SettingsCard>
      )}
    </div>
  );
}

/** Estado de la sincronización: si hay cambios esperando y por qué. */
function CloudSyncText({ sync }: { sync: CloudSyncStatus }) {
  const pending = sync.pending_changes;
  const pendingText = `${pending} cambio${pending === 1 ? "" : "s"} esperando`;
  if (sync.last_error) {
    return (
      <span className="text-amber-400">
        En pausa: {sync.last_error}
        {pending > 0 && ` ${pendingText}; se enviarán solos cuando vuelva la conexión.`}
      </span>
    );
  }
  if (pending > 0) return <span>Enviando… {pendingText}.</span>;
  if (sync.last_synced_at) return <span className="text-emerald-400">Todo sincronizado · {new Date(sync.last_synced_at * 1000).toLocaleTimeString()}</span>;
  return <span>Conectando con la nube…</span>;
}
