import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

const STORAGE_KEY = "vibe-iptv-active-profile-id";

type ActiveProfileContextValue = {
  /** `null` mientras no se ha elegido perfil todavía (o nunca se llegó a elegir uno):
   * el selector de perfiles se enseña en vez del resto de la app hasta que haya uno. */
  profileId: number | null;
  setProfileId: (id: number | null) => void;
};

const ActiveProfileContext = createContext<ActiveProfileContextValue | null>(null);

/** Solo guarda el id, nunca el perfil entero: el nombre/avatar/adult_ok se leen siempre
 * de `GET /api/profiles` (ya cacheado por TanStack Query) para no arrastrar datos
 * obsoletos si el perfil se edita en otra pestaña de la propia app. */
export function ActiveProfileProvider({ children }: { children: ReactNode }) {
  const [profileId, setProfileIdState] = useState<number | null>(null);

  useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) setProfileIdState(Number(raw));
    } catch {
      // localStorage no disponible: el selector de perfiles se enseña siempre, sin más.
    }
  }, []);

  function setProfileId(id: number | null) {
    setProfileIdState(id);
    try {
      if (id === null) localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, String(id));
    } catch {
      // per-sesión nada más: no es grave, solo pedirá perfil otra vez al reabrir.
    }
  }

  return <ActiveProfileContext.Provider value={{ profileId, setProfileId }}>{children}</ActiveProfileContext.Provider>;
}

export function useActiveProfile() {
  const ctx = useContext(ActiveProfileContext);
  if (!ctx) throw new Error("useActiveProfile debe usarse dentro de <ActiveProfileProvider>");
  return ctx;
}
