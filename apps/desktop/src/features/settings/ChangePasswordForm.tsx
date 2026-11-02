import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { apiPost, errorMessage } from "../../lib/api-client";
import { SettingsRow } from "../../components/SettingsRow";
import { BUTTON, INPUT } from "./styles";

/** Cambiar la contraseña de la cuenta con la sesión abierta. La API vuelve a comprobar la
 * actual contra la nube antes de fijar la nueva. */
export function ChangePasswordForm() {
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
      setError(errorMessage(err));
    },
  });
  const ready = currentPassword !== "" && newPassword.length >= 6 && !mutation.isPending;

  return (
    <SettingsRow title="Cambiar contraseña" description="La nueva, de al menos 6 caracteres." stacked>
      <div className="flex w-full flex-col gap-3">
        <div className="flex flex-col gap-3 sm:flex-row">
          <input type="password" placeholder="Contraseña actual" aria-label="Contraseña actual" className={INPUT} value={currentPassword} onChange={(e) => setCurrentPassword(e.target.value)} />
          <input
            type="password"
            placeholder="Contraseña nueva"
            aria-label="Contraseña nueva"
            className={INPUT}
            value={newPassword}
            onChange={(e) => setNewPassword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && ready) mutation.mutate();
            }}
          />
          <button type="button" className={BUTTON} onClick={() => mutation.mutate()} disabled={!ready}>
            {mutation.isPending ? "…" : "Cambiar"}
          </button>
        </div>
        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}
        {success && <p className="text-sm text-emerald-400">Contraseña actualizada.</p>}
      </div>
    </SettingsRow>
  );
}
