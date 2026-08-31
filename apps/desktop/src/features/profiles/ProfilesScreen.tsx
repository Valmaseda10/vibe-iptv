import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPatch, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import type { ProfileSummary } from "../../lib/api-types";

/** Colores para el círculo del avatar — nada de subir una imagen, como Netflix con sus
 * iconos por defecto: elegir uno de una paleta corta es instantáneo y no necesita
 * ningún almacenamiento de ficheros. Se guarda el valor hex tal cual en `avatar`. */
const AVATAR_COLORS = ["#dc2626", "#ea580c", "#d97706", "#65a30d", "#059669", "#0891b2", "#4f46e5", "#a21caf"];

function avatarColorFor(profile: Pick<ProfileSummary, "avatar" | "name">): string {
  if (profile.avatar && AVATAR_COLORS.includes(profile.avatar)) return profile.avatar;
  // Determinista a partir del nombre, para que un perfil sin color explícito no
  // cambie de color en cada render.
  const index = profile.name.charCodeAt(0) % AVATAR_COLORS.length;
  return AVATAR_COLORS[index] ?? "#dc2626";
}

/** Selector de perfiles a pantalla completa, como Netflix: se enseña en vez del resto
 * de la app (`ActiveProfileGate`) mientras no haya un perfil elegido para esta sesión.
 * También hace de pantalla de administración (crear/editar/borrar) con el botón
 * "Administrar perfiles" — Netflix separa las dos cosas en pantallas distintas, aquí
 * comparten una sola para no duplicar la rejilla de tarjetas. */
export function ProfilesScreen() {
  const queryClient = useQueryClient();
  const { setProfileId } = useActiveProfile();
  const [managing, setManaging] = useState(false);
  const [editing, setEditing] = useState<ProfileSummary | "new" | null>(null);
  const [enteringId, setEnteringId] = useState<number | null>(null);
  const [pinInput, setPinInput] = useState("");
  const [enterError, setEnterError] = useState<string | null>(null);

  const profilesQuery = useQuery({
    queryKey: ["profiles"],
    queryFn: () => apiGet<ProfileSummary[]>("/api/profiles"),
  });

  function invalidate() {
    queryClient.invalidateQueries({ queryKey: ["profiles"] });
  }

  const enterMutation = useMutation({
    mutationFn: ({ id, pin }: { id: number; pin: string | null }) => apiPost<ProfileSummary>(`/api/profiles/${id}/enter`, { pin }),
    onSuccess: (profile) => {
      setEnterError(null);
      setEnteringId(null);
      setPinInput("");
      setProfileId(profile.id);
    },
    onError: () => setEnterError("PIN incorrecto."),
  });

  function selectProfile(profile: ProfileSummary) {
    if (managing) {
      setEditing(profile);
      return;
    }
    if (!profile.has_pin) {
      enterMutation.mutate({ id: profile.id, pin: null });
      return;
    }
    setEnteringId(profile.id);
    setEnterError(null);
    setPinInput("");
  }

  const profiles = profilesQuery.data ?? [];

  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-10 bg-neutral-950 px-6 py-10 text-white">
      <h1 className="text-3xl font-semibold sm:text-4xl">{managing ? "Administrar perfiles" : "¿Quién está viendo?"}</h1>

      <div className="flex flex-wrap items-start justify-center gap-6">
        {profiles.map((profile) => (
          <div key={profile.id} className="flex flex-col items-center gap-2">
            <button
              type="button"
              onClick={() => selectProfile(profile)}
              className="group flex h-28 w-28 items-center justify-center rounded-lg text-4xl font-semibold outline-none ring-offset-4 ring-offset-neutral-950 transition hover:ring-4 hover:ring-white focus-visible:ring-4 focus-visible:ring-red-600 sm:h-36 sm:w-36"
              style={{ backgroundColor: avatarColorFor(profile) }}
            >
              {managing ? "✎" : profile.name.charAt(0).toUpperCase()}
            </button>
            <span className="max-w-[9rem] truncate text-sm text-neutral-300">{profile.name}</span>

            {enteringId === profile.id && (
              <div className="flex flex-col items-center gap-2 rounded bg-neutral-900 p-3">
                <input
                  type="password"
                  inputMode="numeric"
                  autoFocus
                  placeholder="PIN"
                  className="w-24 rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-center"
                  value={pinInput}
                  onChange={(e) => setPinInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") enterMutation.mutate({ id: profile.id, pin: pinInput });
                    if (e.key === "Escape") setEnteringId(null);
                  }}
                />
                <button
                  type="button"
                  className="rounded bg-red-600 px-3 py-1 text-sm hover:bg-red-500"
                  onClick={() => enterMutation.mutate({ id: profile.id, pin: pinInput })}
                >
                  Entrar
                </button>
                {enterError && <p className="text-xs text-red-400">{enterError}</p>}
              </div>
            )}
          </div>
        ))}

        {managing && (
          <div className="flex flex-col items-center gap-2">
            <button
              type="button"
              onClick={() => setEditing("new")}
              className="flex h-28 w-28 items-center justify-center rounded-lg border-2 border-dashed border-neutral-700 text-4xl text-neutral-500 outline-none hover:border-neutral-500 hover:text-neutral-300 focus-visible:border-red-600 sm:h-36 sm:w-36"
            >
              +
            </button>
            <span className="text-sm text-neutral-500">Añadir perfil</span>
          </div>
        )}
      </div>

      <button
        type="button"
        className="rounded border border-neutral-700 px-6 py-2 text-neutral-300 outline-none hover:border-neutral-500 hover:text-white focus-visible:border-red-600"
        onClick={() => setManaging((m) => !m)}
      >
        {managing ? "Listo" : "Administrar perfiles"}
      </button>

      {editing && (
        <ProfileEditForm
          profile={editing === "new" ? null : editing}
          canDelete={profiles.length > 1}
          onClose={() => setEditing(null)}
          onSaved={invalidate}
        />
      )}
    </div>
  );
}

function ProfileEditForm({
  profile,
  canDelete,
  onClose,
  onSaved,
}: {
  profile: ProfileSummary | null;
  canDelete: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(profile?.name ?? "");
  const [avatar, setAvatar] = useState(profile ? avatarColorFor(profile) : "#dc2626");
  const [adultOk, setAdultOk] = useState(profile?.adult_ok ?? false);
  const [pin, setPin] = useState("");
  const [clearPin, setClearPin] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (profile) {
        await apiPatch(`/api/profiles/${profile.id}`, { name, avatar, adult_ok: adultOk });
        if (pin) await apiPost(`/api/profiles/${profile.id}/pin`, { pin });
        else if (clearPin) await apiPost(`/api/profiles/${profile.id}/pin`, { pin: null });
      } else {
        const created = await apiPost<ProfileSummary>("/api/profiles", { name, avatar, adult_ok: adultOk });
        if (pin) await apiPost(`/api/profiles/${created.id}/pin`, { pin });
      }
    },
    onSuccess: () => {
      onSaved();
      onClose();
    },
    onError: (err) => setError(String(err)),
  });

  const deleteMutation = useMutation({
    mutationFn: () => apiDelete(`/api/profiles/${profile!.id}`),
    onSuccess: () => {
      onSaved();
      onClose();
    },
    onError: () => setError("No se pudo borrar (¿es el único perfil que queda?)."),
  });

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/70 p-6" onClick={onClose}>
      <div className="w-full max-w-sm rounded-lg bg-neutral-900 p-6" onClick={(e) => e.stopPropagation()}>
        <h2 className="mb-4 text-xl font-medium">{profile ? "Editar perfil" : "Nuevo perfil"}</h2>

        <label className="mb-3 block">
          <span className="mb-1 block text-sm text-neutral-400">Nombre</span>
          <input
            className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </label>

        <div className="mb-3">
          <span className="mb-1 block text-sm text-neutral-400">Color</span>
          <div className="flex gap-2">
            {AVATAR_COLORS.map((color) => (
              <button
                key={color}
                type="button"
                aria-label={`Elegir color ${color}`}
                className={`h-8 w-8 rounded-full outline-none ${avatar === color ? "ring-2 ring-white ring-offset-2 ring-offset-neutral-900" : ""}`}
                style={{ backgroundColor: color }}
                onClick={() => setAvatar(color)}
              />
            ))}
          </div>
        </div>

        <label className="mb-3 flex items-center gap-2">
          <input type="checkbox" checked={adultOk} onChange={(e) => setAdultOk(e.target.checked)} />
          <span>Perfil de adulto (ve contenido marcado como adulto)</span>
        </label>

        <label className="mb-1 block">
          <span className="mb-1 block text-sm text-neutral-400">
            {profile?.has_pin ? "Nuevo PIN (deja en blanco para no cambiarlo)" : "PIN (opcional)"}
          </span>
          <input
            type="password"
            inputMode="numeric"
            className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={pin}
            onChange={(e) => setPin(e.target.value)}
          />
        </label>
        {profile?.has_pin && (
          <label className="mb-3 flex items-center gap-2 text-sm text-neutral-400">
            <input type="checkbox" checked={clearPin} onChange={(e) => setClearPin(e.target.checked)} />
            <span>Quitar el PIN de este perfil</span>
          </label>
        )}

        {error && <p className="mb-3 text-sm text-red-400">{error}</p>}

        <div className="flex items-center justify-between gap-3">
          <div className="flex gap-3">
            <button
              type="button"
              className="rounded bg-red-600 px-4 py-2 hover:bg-red-500 disabled:opacity-50"
              onClick={() => saveMutation.mutate()}
              disabled={!name.trim() || saveMutation.isPending}
            >
              Guardar
            </button>
            <button type="button" className="rounded bg-neutral-700 px-4 py-2 hover:bg-neutral-600" onClick={onClose}>
              Cancelar
            </button>
          </div>
          {profile && canDelete && (
            <button
              type="button"
              className="rounded px-3 py-2 text-sm text-red-400 hover:bg-neutral-800"
              onClick={() => deleteMutation.mutate()}
              disabled={deleteMutation.isPending}
            >
              Eliminar perfil
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
