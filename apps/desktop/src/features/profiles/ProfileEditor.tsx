import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useMutation } from "@tanstack/react-query";
import { ApiError, apiDelete, apiPatch, apiPost, errorMessage } from "../../lib/api-client";
import { AVATAR_GRADIENTS, formatAvatar, parseAvatar, type Avatar } from "../../lib/avatar";
import { FLUENT_AVATARS } from "../../lib/fluentAvatars.generated";
import type { ProfileSummary } from "../../lib/api-types";
import { AvatarImage } from "../../components/AvatarImage";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { SegmentedChoice } from "../../components/SegmentedChoice";
import { Toggle } from "../../components/Toggle";
import { AvatarPicker } from "./AvatarPicker";

const AGE_OPTIONS: readonly { value: number; label: string }[] = [
  { value: -1, label: "Sin límite" },
  { value: 16, label: "16" },
  { value: 12, label: "12" },
  { value: 7, label: "7" },
  { value: 0, label: "TP" },
];

const INPUT =
  "w-full rounded-lg bg-black/40 px-3 py-2.5 text-white outline-none ring-1 ring-white/10 placeholder:text-neutral-500 focus:ring-white/40 ios:rounded-xl ios:bg-ios-surface-2 ios:ring-0";

/** Crear o editar un perfil a pantalla completa, como en Netflix (2026-11-02): la imagen en
 * grande con todo lo que se puede elegir debajo, el nombre, si es infantil y hasta qué
 * edad, el PIN y, abajo del todo, borrarlo (con confirmación). */
export function ProfileEditor({
  profile,
  canDelete,
  catalogProfileId,
  onClose,
  onSaved,
}: {
  profile: ProfileSummary | null;
  canDelete: boolean;
  catalogProfileId: number | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [avatar, setAvatar] = useState<Avatar>(() =>
    profile ? parseAvatar(profile) : { kind: "fluent", slug: FLUENT_AVATARS[Math.floor(Math.random() * FLUENT_AVATARS.length)]?.slug ?? "popcorn", bg: AVATAR_GRADIENTS[0]?.id ?? "#dc2626" },
  );
  const [name, setName] = useState(profile?.name ?? "");
  const [adultOk, setAdultOk] = useState(profile?.adult_ok ?? false);
  // Sin límite por defecto: `adult_ok` apagado solo quita las categorías XXX, no hace
  // infantil el perfil. El límite de edad se elige aquí a propósito (F14).
  const [maxAge, setMaxAge] = useState<number | null>(profile ? profile.max_age : null);
  const [pin, setPin] = useState("");
  const [clearPin, setClearPin] = useState(false);
  const [currentPin, setCurrentPin] = useState("");
  const [adultPin, setAdultPin] = useState("");
  const [needsAdultPin, setNeedsAdultPin] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const changingPin = Boolean(profile?.has_pin) && (pin.length > 0 || clearPin);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const save = useMutation({
    mutationFn: async () => {
      const body = { name: name.trim(), avatar: formatAvatar(avatar), adult_ok: adultOk, max_age: maxAge, adult_pin: adultPin || null };
      if (profile) {
        await apiPatch(`/api/profiles/${profile.id}`, body);
        const current_pin = currentPin || adultPin || null;
        if (pin) await apiPost(`/api/profiles/${profile.id}/pin`, { pin, current_pin });
        else if (clearPin) await apiPost(`/api/profiles/${profile.id}/pin`, { pin: null, current_pin });
      } else {
        const created = await apiPost<ProfileSummary>("/api/profiles", body);
        if (pin) await apiPost(`/api/profiles/${created.id}/pin`, { pin });
      }
    },
    onSuccess: () => {
      onSaved();
      onClose();
    },
    onError: (err) => {
      // 403: el cambio deja ver más y hace falta el PIN de un adulto (F14).
      if (err instanceof ApiError && err.status === 403) setNeedsAdultPin(true);
      setError(errorMessage(err));
    },
  });

  const remove = useMutation({
    mutationFn: () => apiDelete(`/api/profiles/${profile?.id ?? 0}`),
    onSuccess: () => {
      onSaved();
      onClose();
    },
    onError: () => setError("No se pudo borrar (¿es el único perfil que queda?)."),
  });

  return createPortal(
    <div role="dialog" aria-modal="true" aria-label={profile ? "Editar perfil" : "Nuevo perfil"} className="fixed inset-0 z-[70] overflow-y-auto bg-neutral-950 text-white ios:bg-black">
      <header className="sticky top-0 z-10 flex items-center justify-between gap-3 border-b border-white/[0.06] bg-neutral-950/85 px-4 pb-3 pt-[max(0.75rem,env(safe-area-inset-top))] backdrop-blur-xl sm:px-8 ios:bg-black/80">
        <button type="button" onClick={onClose} className="rounded-full px-3 py-1.5 text-[15px] text-neutral-300 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-white/70">
          Cancelar
        </button>
        <h2 className="text-[17px] font-semibold">{profile ? "Editar perfil" : "Nuevo perfil"}</h2>
        <button
          type="button"
          onClick={() => save.mutate()}
          disabled={!name.trim() || save.isPending}
          className="rounded-full bg-white px-4 py-1.5 text-[15px] font-semibold text-black outline-none hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-white/70 disabled:opacity-40"
        >
          {save.isPending ? "Guardando…" : "Guardar"}
        </button>
      </header>

      <div className="mx-auto flex max-w-3xl flex-col gap-8 px-4 py-8 sm:px-8">
        <div className="flex flex-col items-center gap-4">
          <AvatarImage avatar={avatar} name={name || "?"} className="h-32 w-32 rounded-2xl text-5xl shadow-2xl shadow-black/60 sm:h-40 sm:w-40" />
          <input
            className={`${INPUT} max-w-sm text-center text-lg font-semibold`}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Nombre"
            aria-label="Nombre del perfil"
            maxLength={30}
            autoFocus={!profile}
          />
        </div>

        <SettingsCard title="Imagen del perfil">
          <div className="p-4">
            <AvatarPicker value={avatar} name={name} catalogProfileId={profile?.id ?? catalogProfileId} onChange={setAvatar} />
          </div>
        </SettingsCard>

        <SettingsCard
          title="Qué puede ver"
          footer={maxAge !== null ? "Solo verá películas y series con esa clasificación o menos. Lo que no tiene clasificación conocida no se le muestra." : undefined}
        >
          <SettingsRow inline title="Perfil de adulto" description="Ve las categorías marcadas como para adultos.">
            <Toggle checked={adultOk} onChange={setAdultOk} label="Perfil de adulto" />
          </SettingsRow>
          <SettingsRow title="Edad máxima" description="Para perfiles infantiles.">
            <SegmentedChoice label="Edad máxima" options={AGE_OPTIONS} value={maxAge ?? -1} onChange={(v) => setMaxAge(v === -1 ? null : v)} />
          </SettingsRow>
        </SettingsCard>

        <SettingsCard title="PIN" footer="Con PIN, hay que escribirlo para entrar en este perfil.">
          <SettingsRow title={profile?.has_pin ? "Cambiar PIN" : "PIN de entrada"} description={profile?.has_pin ? "Déjalo en blanco para no cambiarlo." : "Opcional, de 4 cifras."} stacked>
            <input type="password" inputMode="numeric" maxLength={8} className={`${INPUT} max-w-[12rem]`} value={pin} onChange={(e) => setPin(e.target.value)} aria-label="PIN nuevo" placeholder="••••" />
          </SettingsRow>
          {profile?.has_pin && (
            <SettingsRow inline title="Quitar el PIN" description="Cualquiera podrá entrar en este perfil.">
              <Toggle checked={clearPin} onChange={setClearPin} label="Quitar el PIN" />
            </SettingsRow>
          )}
          {changingPin && (
            <SettingsRow title="PIN actual" description="El de este perfil o el de un adulto." stacked>
              <input type="password" inputMode="numeric" className={`${INPUT} max-w-[12rem]`} value={currentPin} onChange={(e) => setCurrentPin(e.target.value)} aria-label="PIN actual" />
            </SettingsRow>
          )}
          {needsAdultPin && (
            <SettingsRow title="PIN de un adulto" description="Para dejar que este perfil vea más." stacked>
              <input type="password" inputMode="numeric" autoFocus className={`${INPUT} max-w-[12rem] ring-amber-500/60`} value={adultPin} onChange={(e) => setAdultPin(e.target.value)} aria-label="PIN de un adulto" />
            </SettingsRow>
          )}
        </SettingsCard>

        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}

        {profile && canDelete && (
          <div className="flex flex-col items-center gap-3 pb-10">
            {confirmDelete ? (
              <>
                <p className="text-center text-sm text-neutral-300">Se borrarán su Mi lista, su progreso y sus gustos, también en tus otros aparatos.</p>
                <div className="flex gap-2">
                  <button type="button" onClick={() => setConfirmDelete(false)} className="rounded-full bg-white/10 px-4 py-2 text-sm font-medium outline-none hover:bg-white/15 focus-visible:ring-2 focus-visible:ring-white/70">
                    Cancelar
                  </button>
                  <button
                    type="button"
                    onClick={() => remove.mutate()}
                    disabled={remove.isPending}
                    className="rounded-full bg-red-600 px-4 py-2 text-sm font-semibold outline-none hover:bg-red-500 focus-visible:ring-2 focus-visible:ring-white/70 disabled:opacity-50"
                  >
                    Sí, borrar «{profile.name}»
                  </button>
                </div>
              </>
            ) : (
              <button type="button" onClick={() => setConfirmDelete(true)} className="rounded-full px-4 py-2 text-sm font-medium text-red-400 outline-none hover:bg-red-950/50 focus-visible:ring-2 focus-visible:ring-white/70">
                Borrar perfil
              </button>
            )}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
