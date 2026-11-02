import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { RELEASE_NOTES } from "./releaseNotes.generated";

export { RELEASE_NOTES };

const SEEN_KEY = "vibe.whatsNew.seen";

/** La versión instalada (la del núcleo, la misma en PC e iPhone). */
export function useAppVersion(): string | null {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);
  return version;
}

/** «Novedades» al abrir por primera vez una versión nueva (no al instalar la app: ahí todo
 * es nuevo). Devuelve si hay que enseñarlas y cómo darlas por vistas. */
export function useWhatsNewOnLaunch(): [boolean, () => void] {
  const version = useAppVersion();
  const [show, setShow] = useState(false);
  useEffect(() => {
    if (!version) return;
    let seen: string | null = null;
    try {
      seen = window.localStorage.getItem(SEEN_KEY);
      if (seen === null) window.localStorage.setItem(SEEN_KEY, version);
    } catch {
      return;
    }
    if (seen !== null && seen !== version && RELEASE_NOTES.version === version && RELEASE_NOTES.notes.length > 0) setShow(true);
  }, [version]);
  function dismiss() {
    setShow(false);
    try {
      if (version) window.localStorage.setItem(SEEN_KEY, version);
    } catch {
      // Sin almacenamiento: volverán a salir la próxima vez.
    }
  }
  return [show, dismiss];
}
