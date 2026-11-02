import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { parseAvatar } from "../../lib/avatar";
import { AvatarImage } from "../../components/AvatarImage";
import { BrandMark } from "../../components/BrandMark";
import { ProfileEditor } from "./ProfileEditor";
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

  // Recién iniciada la sesión en un dispositivo nuevo, los perfiles de la cuenta pueden
  // estar llegando todavía de la nube: mientras no haya ninguno, se vuelve a mirar.
  const profilesQuery = useQuery({
    queryKey: ["profiles"],
    queryFn: () => apiGet<ProfileSummary[]>("/api/profiles"),
    refetchInterval: (query) => (query.state.data?.length === 0 && query.state.dataUpdateCount < 15 ? 2000 : false),
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
      <div className="flex h-full min-h-screen flex-col items-center justify-center gap-6 bg-neutral-950 px-6 py-10 text-white ios:bg-black">
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
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-10 bg-[radial-gradient(ellipse_at_top,rgba(255,45,85,0.10),transparent_60%)] bg-neutral-950 px-6 py-10 text-white ios:bg-black">
      <BrandMark className="h-10 w-10" />
      <h1 className="text-3xl font-semibold sm:text-5xl sm:font-bold sm:tracking-tight">{managing ? "Administrar perfiles" : "¿Quién está viendo?"}</h1>

      <div className="flex flex-wrap items-start justify-center gap-6">
        {profiles.map((profile) => {
          return (
            <div key={profile.id} className="flex flex-col items-center gap-3">
              <button
                type="button"
                onClick={() => selectProfile(profile)}
                aria-label={managing ? `Editar el perfil ${profile.name}` : `Entrar como ${profile.name}`}
                className="group relative rounded-2xl outline-none ring-offset-4 ring-offset-neutral-950 transition duration-200 hover:scale-105 hover:ring-4 hover:ring-white focus-visible:scale-105 focus-visible:ring-4 focus-visible:ring-white ios:ring-offset-black"
              >
                <AvatarImage avatar={parseAvatar(profile)} name={profile.name} className="h-28 w-28 rounded-2xl text-5xl sm:h-40 sm:w-40 sm:text-6xl" />
                {managing && (
                  <span className="absolute inset-0 flex items-center justify-center rounded-2xl bg-black/50 text-3xl text-white">
                    <span className="flex h-12 w-12 items-center justify-center rounded-full ring-2 ring-white/80">✎</span>
                  </span>
                )}
                {profile.has_pin && !managing && <span className="absolute bottom-2 right-2 rounded-full bg-black/60 px-1.5 py-0.5 text-[11px]">🔒</span>}
              </button>
              <span className="max-w-[10rem] truncate text-base text-neutral-300 group-hover:text-white">{profile.name}</span>

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
              className="flex h-28 w-28 items-center justify-center rounded-2xl bg-white/[0.04] text-5xl text-neutral-500 outline-none ring-2 ring-inset ring-white/10 transition hover:bg-white/[0.08] hover:text-white focus-visible:ring-white sm:h-40 sm:w-40"
            >
              +
            </button>
            <span className="text-base text-neutral-500">Añadir perfil</span>
          </div>
        )}
      </div>

      <button
        type="button"
        className={`rounded-full px-6 py-2.5 font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-white ${managing ? "bg-white text-black hover:bg-neutral-200" : "text-neutral-300 ring-1 ring-white/25 hover:text-white hover:ring-white/60"}`}
        onClick={() => setManaging((m) => !m)}
      >
        {managing ? "Listo" : "Administrar perfiles"}
      </button>

      {editing && (
        <ProfileEditor
          profile={editing === "new" ? null : editing}
          canDelete={profiles.length > 1}
          catalogProfileId={profiles[0]?.id ?? null}
          onClose={() => setEditing(null)}
          onSaved={invalidate}
        />
      )}
    </div>
  );
}
