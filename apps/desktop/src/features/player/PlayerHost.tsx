import { useCallback, useEffect, useMemo, useRef } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { IS_IOS } from "../../lib/platform";
import { useActiveProfile } from "../../lib/activeProfile";
import { sessionKey, usePlayerSession, type PlayerKind, type PlayerSearch, type PlayerSession } from "../../lib/playerSession";
import { PlayerScreen } from "./PlayerScreen";

/** El reproductor del PC, montado por encima de toda la app (no dentro de la ruta).
 *
 * En `/player/...` ocupa la pantalla entera. Al salir de ahí (Volver, Esc, el menú) sigue
 * reproduciéndose en una ventana pequeña abajo a la derecha, como en YouTube o Netflix,
 * hasta que se cierra con ✕ o se abre otro título o canal. Pinchar en ella la devuelve a
 * pantalla completa. Es el mismo `<video>` todo el rato: no se corta ni se vuelve a pedir
 * el stream al cambiar de tamaño (regla R1).
 *
 * En iPhone y iPad no aplica: allí reproduce VLC en una pantalla nativa. */
export function PlayerHost() {
  const { session, setSession } = usePlayerSession();
  const navigate = useNavigate();
  const { profileId } = useActiveProfile();
  const playerMatch = useRouterState({
    select: (state) => state.matches.find((match) => match.routeId === "/player/$kind/$id"),
  });
  const fromUrl: PlayerSession | null = playerMatch
    ? {
        kind: (playerMatch.params as { kind: PlayerKind }).kind,
        id: (playerMatch.params as { id: string }).id,
        search: playerMatch.search as PlayerSearch,
      }
    : null;
  const urlKey = sessionKey(fromUrl);
  // Mismo objeto mientras no cambie de verdad (la URL se vuelve a leer en cada render).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const urlSession = useMemo(() => fromUrl, [urlKey]);
  const expanded = urlSession !== null;

  // Lo que se ve a pantalla completa queda como sesión para cuando se salga.
  useEffect(() => {
    if (urlSession && sessionKey(urlSession) !== sessionKey(session)) setSession(urlSession);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [urlSession]);

  // Cambiar de perfil cierra lo que estaba viendo el anterior.
  const profileRef = useRef(profileId);
  useEffect(() => {
    if (profileRef.current === profileId) return;
    profileRef.current = profileId;
    setSession(null);
  }, [profileId, setSession]);

  const active = urlSession ?? session;

  // Cambios desde dentro del reproductor (zapping, otra fuente, siguiente episodio): a
  // pantalla completa van por la URL; en pequeño solo cambian la sesión, sin sacar al
  // usuario de la pantalla en la que está.
  const go = useCallback(
    (next: PlayerSession, options?: { replace?: boolean }) => {
      if (expanded) navigate({ to: "/player/$kind/$id", params: { kind: next.kind, id: next.id }, search: next.search, replace: options?.replace });
      else setSession(next);
    },
    [expanded, navigate, setSession],
  );
  const expand = useCallback(() => {
    if (!session) return;
    navigate({ to: "/player/$kind/$id", params: { kind: session.kind, id: session.id }, search: session.search });
  }, [navigate, session]);
  const close = useCallback(() => setSession(null), [setSession]);

  if (IS_IOS || !active) return null;

  // Cada título es un reproductor nuevo; el directo es uno solo aunque se cambie de canal
  // (zapping, número tecleado, «0» para el anterior...).
  const key = active.kind === "channel" ? "channel" : `${active.kind}:${active.id}`;
  return (
    <div
      className={
        expanded
          ? "fixed inset-0 z-50 bg-black"
          : "fixed bottom-20 right-4 z-40 aspect-video w-[min(360px,calc(100vw-2rem))] overflow-hidden rounded-xl bg-black shadow-2xl shadow-black/60 ring-1 ring-white/15 desk:bottom-4"
      }
      role={expanded ? undefined : "region"}
      aria-label={expanded ? undefined : "Reproductor en miniatura"}
    >
      <PlayerScreen key={key} kind={active.kind} id={active.id} search={active.search} mini={!expanded} onNavigate={go} onExpand={expand} onClose={close} />
    </div>
  );
}
