import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiDelete, apiGet, apiPatch, apiPost, errorMessage } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { AVATAR_COLORS, AVATAR_ICONS, formatAvatar, parseAvatar } from "../../lib/avatar";
import type { ProfileSummary } from "../../lib/api-types";

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

  // Cuenta recién creada (o recién vaciada): en vez de la rejilla habitual con un botón
  // "Administrar perfiles" escondido, se invita directamente a crear el primer perfil —
  // así se cumple lo que se le dice al usuario nada más entrar: "crea un perfil".
  if (!profilesQuery.isPending && profiles.length === 0 && editing !== "new") {
    return (
      <div className="flex h-full min-h-screen flex-col items-center justify-center gap-6 bg-neutral-950 px-6 py-10 text-white">
        <h1 className="text-3xl font-semibold sm:text-4xl">Crea tu primer perfil</h1>
        <p className="max-w-sm text-center text-neutral-400">
          Cada perfil tiene sus propios favoritos, su "Continuar viendo" y su control de contenido para adultos.
        </p>
        <button
          type="button"
          onClick={() => setEditing("new")}
          aria-label="Añadir perfil"
          className="flex h-28 w-28 items-center justify-center rounded-lg border-2 border-dashed border-neutral-700 text-4xl text-neutral-500 outline-none hover:border-neutral-500 hover:text-neutral-300 focus-visible:border-red-600 sm:h-36 sm:w-36"
        >
          +
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-10 bg-neutral-950 px-6 py-10 text-white">
      <h1 className="text-3xl font-semibold sm:text-4xl">{managing ? "Administrar perfiles" : "¿Quién está viendo?"}</h1>

      <div className="flex flex-wrap items-start justify-center gap-6">
        {profiles.map((profile) => {
          const { icon, color } = parseAvatar(profile);
          return (
            <div key={profile.id} className="flex flex-col items-center gap-2">
              <button
                type="button"
                onClick={() => selectProfile(profile)}
                aria-label={managing ? `Editar el perfil ${profile.name}` : `Entrar como ${profile.name}`}
                className="group relative flex h-28 w-28 items-center justify-center rounded-lg text-4xl font-semibold outline-none ring-offset-4 ring-offset-neutral-950 transition hover:ring-4 hover:ring-white focus-visible:ring-4 focus-visible:ring-red-600 sm:h-36 sm:w-36"
                style={{ backgroundColor: color }}
              >
                {managing ? <span className="absolute inset-0 flex items-center justify-center bg-black/40 text-2xl">✎</span> : null}
                {icon ?? profile.name.charAt(0).toUpperCase()}
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
          );
        })}

        {managing && (
          <div className="flex flex-col items-center gap-2">
            <button
              type="button"
              onClick={() => setEditing("new")}
              aria-label="Añadir perfil"
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
  const initial = profile ? parseAvatar(profile) : { icon: AVATAR_ICONS[0] ?? null, color: AVATAR_COLORS[0] ?? "#dc2626" };
  const [name, setName] = useState(profile?.name ?? "");
  const [icon, setIcon] = useState<string | null>(initial.icon);
  const [color, setColor] = useState(initial.color);
  const [adultOk, setAdultOk] = useState(profile?.adult_ok ?? false);
  // Sin límite por defecto: `adult_ok` desmarcado solo quita las categorías XXX, no hace
  // infantil el perfil. El límite de edad se elige aquí a propósito (F14).
  const [maxAge, setMaxAge] = useState<number | null>(profile ? profile.max_age : null);
  const [pin, setPin] = useState("");
  const [clearPin, setClearPin] = useState(false);
  const [currentPin, setCurrentPin] = useState("");
  const [adultPin, setAdultPin] = useState("");
  const [needsAdultPin, setNeedsAdultPin] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const changingPin = Boolean(profile?.has_pin) && (pin.length > 0 || clearPin);

  const saveMutation = useMutation({
    mutationFn: async () => {
      const avatar = formatAvatar(icon, color);
      const body = { name, avatar, adult_ok: adultOk, max_age: maxAge, adult_pin: adultPin || null };
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

  const deleteMutation = useMutation({
    mutationFn: () => apiDelete(`/api/profiles/${profile!.id}`),
    onSuccess: () => {
      onSaved();
      onClose();
    },
    onError: () => setError("No se pudo borrar (¿es el único perfil que queda?)."),
  });

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center overflow-y-auto bg-black/70 p-6" onClick={onClose}>
      <div className="w-full max-w-sm rounded-lg bg-neutral-900 p-6" onClick={(e) => e.stopPropagation()}>
        <h2 className="mb-4 text-xl font-medium">{profile ? "Editar perfil" : "Nuevo perfil"}</h2>

        <div className="mb-4 flex items-center justify-center">
          <div className="flex h-20 w-20 items-center justify-center rounded-lg text-3xl" style={{ backgroundColor: color }}>
            {icon ?? name.charAt(0).toUpperCase() ?? "?"}
          </div>
        </div>

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
          <span className="mb-1 block text-sm text-neutral-400">Icono</span>
          <div className="flex flex-wrap gap-2">
            {AVATAR_ICONS.map((candidate) => (
              <button
                key={candidate}
                type="button"
                aria-label={`Elegir icono ${candidate}`}
                aria-pressed={icon === candidate}
                className={`flex h-9 w-9 items-center justify-center rounded text-lg outline-none ${
                  icon === candidate ? "bg-neutral-700 ring-2 ring-white" : "bg-neutral-800 hover:bg-neutral-700"
                }`}
                onClick={() => setIcon(icon === candidate ? null : candidate)}
              >
                {candidate}
              </button>
            ))}
          </div>
        </div>

        <div className="mb-3">
          <span className="mb-1 block text-sm text-neutral-400">Color de fondo</span>
          <div className="flex gap-2">
            {AVATAR_COLORS.map((candidate) => (
              <button
                key={candidate}
                type="button"
                aria-label={`Elegir color ${candidate}`}
                aria-pressed={color === candidate}
                className={`h-8 w-8 rounded-full outline-none ${color === candidate ? "ring-2 ring-white ring-offset-2 ring-offset-neutral-900" : ""}`}
                style={{ backgroundColor: candidate }}
                onClick={() => setColor(candidate)}
              />
            ))}
          </div>
        </div>

        <label className="mb-3 flex items-center gap-2">
          <input type="checkbox" checked={adultOk} onChange={(e) => setAdultOk(e.target.checked)} />
          <span>Perfil de adulto (ve contenido marcado como adulto)</span>
        </label>

        <label className="mb-3 block">
          <span className="mb-1 block text-sm text-neutral-400">Edad máxima de lo que puede ver</span>
          <select
            className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
            value={maxAge ?? ""}
            onChange={(e) => setMaxAge(e.target.value === "" ? null : Number(e.target.value))}
          >
            <option value="">Sin límite</option>
            <option value={16}>Hasta 16 años</option>
            <option value={12}>Hasta 12 años</option>
            <option value={7}>Hasta 7 años</option>
            <option value={0}>Solo para todos los públicos</option>
          </select>
          {maxAge !== null && (
            <span className="mt-1 block text-xs text-neutral-500">
              Solo verá películas y series con esa clasificación o menos. Lo que no tiene clasificación conocida no se le
              muestra.
            </span>
          )}
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

        {changingPin && (
          <label className="mb-3 block">
            <span className="mb-1 block text-sm text-neutral-400">PIN actual de este perfil (o el de un adulto)</span>
            <input
              type="password"
              inputMode="numeric"
              className="w-full rounded border border-neutral-700 bg-neutral-950 px-3 py-2"
              value={currentPin}
              onChange={(e) => setCurrentPin(e.target.value)}
            />
          </label>
        )}

        {needsAdultPin && (
          <label className="mb-3 block">
            <span className="mb-1 block text-sm text-amber-300">PIN de un perfil adulto, para dejar que este perfil vea más</span>
            <input
              type="password"
              inputMode="numeric"
              autoFocus
              className="w-full rounded border border-amber-600/60 bg-neutral-950 px-3 py-2"
              value={adultPin}
              onChange={(e) => setAdultPin(e.target.value)}
            />
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
