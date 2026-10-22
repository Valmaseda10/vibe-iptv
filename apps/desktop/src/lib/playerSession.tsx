import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

export type PlayerKind = "channel" | "movie" | "episode";

/** Parámetros de búsqueda de `/player/$kind/$id`. */
export type PlayerSearch = {
  seriesId?: string;
  categoryId?: string;
  /** Epoch (segundos UTC) de inicio, solo si se abre desde la Guía como catch-up (F9)
   * en vez de directo. Su sola presencia es lo que distingue los dos modos. */
  catchupStart?: number;
  /** Minutos a pedir desde `catchupStart`. Si falta, el backend usa un valor por defecto. */
  catchupDuration?: number;
  /** Id de `movie_sources` elegido (selector de fuente de la ficha o del reproductor). */
  source?: number;
  /** Índice relativo de pista de audio elegida. Solo tiene efecto en nivel 2 (remux). */
  audioTrack?: number;
  /** Abierto desde «★ Favoritos» del directo: la lista y el zapping son los favoritos. */
  favorites?: boolean;
};

/** Lo que se está reproduciendo. Vive por encima de las pantallas: al salir del
 * reproductor sigue sonando en pequeño (`PlayerHost`) hasta que se cierra o se abre otra cosa. */
export type PlayerSession = { kind: PlayerKind; id: string; search: PlayerSearch };

type PlayerSessionContextValue = {
  session: PlayerSession | null;
  setSession: (session: PlayerSession | null) => void;
};

const PlayerSessionContext = createContext<PlayerSessionContextValue | null>(null);

export function PlayerSessionProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<PlayerSession | null>(null);
  const value = useMemo(() => ({ session, setSession }), [session]);
  return <PlayerSessionContext.Provider value={value}>{children}</PlayerSessionContext.Provider>;
}

export function usePlayerSession(): PlayerSessionContextValue {
  const value = useContext(PlayerSessionContext);
  if (!value) throw new Error("usePlayerSession fuera de PlayerSessionProvider");
  return value;
}

/** Clave estable de una sesión, para comparar sin depender de la identidad del objeto. */
export function sessionKey(session: PlayerSession | null): string {
  return session ? JSON.stringify([session.kind, session.id, session.search]) : "";
}
