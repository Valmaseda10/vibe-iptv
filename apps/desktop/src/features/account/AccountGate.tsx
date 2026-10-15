import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiGet, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { AccountStatusResponse } from "../../lib/api-types";

type View = "auth" | "forgot" | "reset";

/** Envuelve toda la app, por fuera incluso del selector de perfiles: hace falta cuenta
 * e inicio de sesión, igual que Netflix o Prime Video — los perfiles viven dentro de la
 * cuenta, no antes de ella. No hay ningún botón para saltárselo, con una excepción
 * (F11, local-first): si la nube no responde al intentar entrar, se ofrece entrar sin
 * conexión, porque el catálogo y la reproducción son de este PC y no dependen de ella.
 * Con una sesión ya guardada, la nube caída tampoco echa a nadie: el núcleo responde
 * `logged_in` sin preguntarle. El estado real de sesión es la única fuente de verdad:
 * cerrar sesión invalida esta consulta y la puerta vuelve a aparecer sola. */
export function AccountGate({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const { setProfileId } = useActiveProfile();
  const [offline, setOffline] = useState(false);

  const statusQuery = useQuery({
    queryKey: ["account-status"],
    queryFn: () => apiGet<AccountStatusResponse>("/api/account/status"),
    retry: false,
  });

  function handleLoggedIn() {
    queryClient.invalidateQueries({ queryKey: ["account-status"] });
    // Iniciar sesión es un evento de cuenta nuevo — puede ser otra persona, o el mismo
    // usuario en otro momento — así que se pregunta perfil de cero, igual que Netflix,
    // en vez de arrastrar silenciosamente el último perfil que quedó activo en este
    // dispositivo (que podría ni siquiera pertenecer a esta cuenta).
    setProfileId(null);
    queryClient.invalidateQueries({ queryKey: ["profiles"] });
  }

  if (statusQuery.data?.logged_in || offline) {
    return <>{children}</>;
  }

  // Fallo de red genuino (no "sin sesión", sino "no se pudo ni preguntar"): no bloquea
  // la app por algo que no es culpa del usuario ni tiene botón que lo arregle.
  if (statusQuery.isError) {
    return <>{children}</>;
  }

  if (statusQuery.isPending) {
    return <div className="flex h-full min-h-screen items-center justify-center bg-neutral-950 text-neutral-500">Cargando…</div>;
  }

  return <AccountForm onLoggedIn={handleLoggedIn} onEnterOffline={() => setOffline(true)} />;
}

function AccountForm({ onLoggedIn, onEnterOffline }: { onLoggedIn: () => void; onEnterOffline: () => void }) {
  const [view, setView] = useState<View>("auth");
  const [mode, setMode] = useState<"login" | "signup">("signup");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [cloudDown, setCloudDown] = useState(false);

  const authMutation = useMutation({
    mutationFn: () => apiPost<AccountStatusResponse>(mode === "login" ? "/api/account/login" : "/api/account/signup", { email, password }),
    onSuccess: () => {
      setError(null);
      onLoggedIn();
    },
    onError: (err) => {
      setError(String(err));
      setCloudDown(err instanceof ApiError && err.status === 503);
    },
  });

  if (view === "forgot") {
    return (
      <AuthShell>
        <ForgotPasswordForm email={email} onSent={(sentEmail) => { setEmail(sentEmail); setView("reset"); }} onBack={() => setView("auth")} />
      </AuthShell>
    );
  }

  if (view === "reset") {
    return (
      <AuthShell>
        <ResetPasswordForm email={email} onDone={onLoggedIn} onBack={() => setView("auth")} />
      </AuthShell>
    );
  }

  return (
    <AuthShell>
      <div className="mb-4 flex gap-4" role="radiogroup" aria-label="Iniciar sesión o crear cuenta">
        {(["signup", "login"] as const).map((m) => (
          <label key={m} className="flex items-center gap-2">
            <input type="radio" checked={mode === m} onChange={() => setMode(m)} />
            {m === "signup" ? "Crear cuenta" : "Iniciar sesión"}
          </label>
        ))}
      </div>

      <label className="mb-3 block">
        <span className="mb-1 block text-sm text-neutral-400">Correo</span>
        <input
          type="email"
          autoFocus
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
      </label>
      <label className="mb-2 block">
        <span className="mb-1 block text-sm text-neutral-400">Contraseña</span>
        <input
          type="password"
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && email && password) authMutation.mutate();
          }}
        />
      </label>

      {mode === "login" && (
        <button type="button" className="mb-4 text-xs text-neutral-500 underline decoration-dotted hover:text-neutral-300" onClick={() => setView("forgot")}>
          ¿Olvidaste tu contraseña?
        </button>
      )}

      <button
        type="button"
        className="w-full rounded bg-red-600 px-4 py-2 font-medium hover:bg-red-500 disabled:opacity-50"
        onClick={() => authMutation.mutate()}
        disabled={!email || !password || authMutation.isPending}
      >
        {authMutation.isPending ? "…" : mode === "login" ? "Entrar" : "Crear cuenta"}
      </button>

      {error && <p role="alert" className="mt-3 text-sm text-red-400">{error}</p>}
      {cloudDown && (
        <div className="mt-4 border-t border-neutral-800 pt-4">
          <p className="mb-3 text-sm text-neutral-400">
            Puedes usar la app con lo que ya hay en este PC. Tus cambios se guardan aquí y se sincronizarán cuando inicies
            sesión desde Ajustes.
          </p>
          <button type="button" className="w-full rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={onEnterOffline}>
            Entrar sin conexión
          </button>
        </div>
      )}
    </AuthShell>
  );
}

/** Marco común de las tres pantallas (entrar, pedir recuperación, completarla) — mismo
 * fondo y logo, solo cambia lo de dentro de la tarjeta. */
function AuthShell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-8 bg-neutral-950 px-6 py-10 text-white">
      <div className="flex flex-col items-center gap-2">
        <h1 className="text-3xl font-semibold sm:text-4xl">VIBE IPTV</h1>
        <p className="text-neutral-400">Crea tu cuenta o inicia sesión para ver tus perfiles y que te sigan a otros dispositivos.</p>
      </div>
      <div className="w-full max-w-sm rounded-lg border border-neutral-800 bg-neutral-900 p-6">{children}</div>
    </div>
  );
}

function ForgotPasswordForm({ email: initialEmail, onSent, onBack }: { email: string; onSent: (email: string) => void; onBack: () => void }) {
  const [email, setEmail] = useState(initialEmail);
  const [error, setError] = useState<string | null>(null);

  const sendMutation = useMutation({
    mutationFn: () => apiPost("/api/account/forgot-password", { email }),
    onSuccess: () => {
      setError(null);
      onSent(email);
    },
    onError: (err) => setError(String(err)),
  });

  return (
    <>
      <h2 className="mb-3 text-lg font-medium">Recuperar contraseña</h2>
      <p className="mb-4 text-sm text-neutral-400">
        Te mandamos un correo con un enlace. Ábrelo y elige la contraseña nueva en el navegador, o pégalo aquí en el
        siguiente paso.
      </p>
      <label className="mb-4 block">
        <span className="mb-1 block text-sm text-neutral-400">Correo</span>
        <input
          type="email"
          autoFocus
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && email) sendMutation.mutate();
          }}
        />
      </label>
      <div className="flex gap-3">
        <button
          type="button"
          className="flex-1 rounded bg-red-600 px-4 py-2 font-medium hover:bg-red-500 disabled:opacity-50"
          onClick={() => sendMutation.mutate()}
          disabled={!email || sendMutation.isPending}
        >
          {sendMutation.isPending ? "…" : "Enviar enlace"}
        </button>
        <button type="button" className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={onBack}>
          Volver
        </button>
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-red-400">{error}</p>}
    </>
  );
}

function ResetPasswordForm({ email: initialEmail, onDone, onBack }: { email: string; onDone: () => void; onBack: () => void }) {
  const [email, setEmail] = useState(initialEmail);
  const [tokenOrLink, setTokenOrLink] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  const resetMutation = useMutation({
    mutationFn: () => apiPost<AccountStatusResponse>("/api/account/reset-password", { email, token_or_link: tokenOrLink, new_password: newPassword }),
    onSuccess: () => {
      setError(null);
      onDone();
    },
    onError: (err) => setError(String(err)),
  });

  const canSubmit = email && tokenOrLink.trim() && newPassword.length >= 6;

  return (
    <>
      <h2 className="mb-3 text-lg font-medium">Completa el cambio</h2>
      <p className="mb-4 text-sm text-neutral-400">Pega aquí el enlace completo del correo (o solo el código que trae) y elige tu contraseña nueva.</p>
      <label className="mb-3 block">
        <span className="mb-1 block text-sm text-neutral-400">Correo</span>
        <input
          type="email"
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
      </label>
      <label className="mb-3 block">
        <span className="mb-1 block text-sm text-neutral-400">Enlace o código del correo</span>
        <input
          type="text"
          autoFocus
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={tokenOrLink}
          onChange={(e) => setTokenOrLink(e.target.value)}
        />
      </label>
      <label className="mb-4 block">
        <span className="mb-1 block text-sm text-neutral-400">Contraseña nueva (mínimo 6 caracteres)</span>
        <input
          type="password"
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
          value={newPassword}
          onChange={(e) => setNewPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && canSubmit) resetMutation.mutate();
          }}
        />
      </label>
      <div className="flex gap-3">
        <button
          type="button"
          className="flex-1 rounded bg-red-600 px-4 py-2 font-medium hover:bg-red-500 disabled:opacity-50"
          onClick={() => resetMutation.mutate()}
          disabled={!canSubmit || resetMutation.isPending}
        >
          {resetMutation.isPending ? "…" : "Restablecer contraseña"}
        </button>
        <button type="button" className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={onBack}>
          Volver
        </button>
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-red-400">{error}</p>}
    </>
  );
}
