import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { AccountStatusResponse } from "../../lib/api-types";

/** Envuelve toda la app, por fuera incluso del selector de perfiles: hace falta cuenta
 * e inicio de sesión siempre, igual que Netflix o Prime Video — los perfiles viven
 * dentro de la cuenta, no antes de ella. No hay ningún botón para saltárselo: la única
 * excepción es un fallo de red genuino (ni siquiera se pudo preguntar el estado de la
 * cuenta a la propia app local), que deja pasar en silencio para no dejar la app
 * completamente inutilizable por algo que no tiene botón que lo arregle. El estado real
 * de sesión (`logged_in`) es la única fuente de verdad: cerrar sesión invalida esta
 * consulta y la puerta vuelve a aparecer sola, sin ninguna bandera "no preguntar más"
 * que recordar ni deshacer. */
export function AccountGate({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const { setProfileId } = useActiveProfile();

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

  if (statusQuery.data?.logged_in) {
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

  return <AccountForm onLoggedIn={handleLoggedIn} />;
}

function AccountForm({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [mode, setMode] = useState<"login" | "signup">("signup");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const authMutation = useMutation({
    mutationFn: () => apiPost<AccountStatusResponse>(mode === "login" ? "/api/account/login" : "/api/account/signup", { email, password }),
    onSuccess: (result) => {
      setError(null);
      if (result.logged_in) {
        onLoggedIn();
      } else {
        setInfo("Cuenta creada. Revisa tu correo para confirmarla y luego inicia sesión.");
        setPassword("");
      }
    },
    onError: (err) => setError(String(err)),
  });

  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-8 bg-neutral-950 px-6 py-10 text-white">
      <div className="flex flex-col items-center gap-2">
        <h1 className="text-3xl font-semibold sm:text-4xl">VIBE IPTV</h1>
        <p className="text-neutral-400">Crea tu cuenta o inicia sesión para ver tus perfiles y que te sigan a otros dispositivos.</p>
      </div>

      <div className="w-full max-w-sm rounded-lg border border-neutral-800 bg-neutral-900 p-6">
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
        <label className="mb-4 block">
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

        <button
          type="button"
          className="w-full rounded bg-red-600 px-4 py-2 font-medium hover:bg-red-500 disabled:opacity-50"
          onClick={() => authMutation.mutate()}
          disabled={!email || !password || authMutation.isPending}
        >
          {authMutation.isPending ? "…" : mode === "login" ? "Entrar" : "Crear cuenta"}
        </button>

        {error && <p className="mt-3 text-sm text-red-400">{error}</p>}
        {info && <p className="mt-3 text-sm text-neutral-300">{info}</p>}
      </div>
    </div>
  );
}
