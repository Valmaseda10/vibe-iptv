import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { apiGet } from "../../lib/api-client";
import { useActiveProfile } from "../../lib/activeProfile";
import { parseAvatar } from "../../lib/avatar";
import type { ProfileSummary } from "../../lib/api-types";

/** Icono del perfil activo, arriba a la izquierda de la barra lateral — igual que el
 * avatar de Netflix en su esquina: siempre a la vista, un clic para cambiar de perfil o
 * ir a ajustes, sin tener que entrar primero en Ajustes para encontrar "Cambiar de
 * perfil". */
export function ProfileSwitcher({ collapsed }: { collapsed: boolean }) {
  const { profileId, setProfileId } = useActiveProfile();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const profilesQuery = useQuery({
    queryKey: ["profiles"],
    queryFn: () => apiGet<ProfileSummary[]>("/api/profiles"),
  });

  const active = profilesQuery.data?.find((p) => p.id === profileId) ?? null;

  useEffect(() => {
    if (!open) return;
    function handlePointerDown(event: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) setOpen(false);
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  if (!active) return null;

  const { icon, color } = parseAvatar(active);

  return (
    <div ref={rootRef} className="relative px-2">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        title={active.name}
        className={`flex items-center gap-2 rounded-lg py-1.5 outline-none transition-colors hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-red-600 ${
          collapsed ? "w-full justify-center px-0" : "w-full px-2"
        }`}
      >
        <span
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded text-sm font-semibold"
          style={{ backgroundColor: color }}
        >
          {icon ?? active.name.charAt(0).toUpperCase()}
        </span>
        {!collapsed && <span className="truncate text-sm text-neutral-200">{active.name}</span>}
      </button>

      {open && (
        <div
          role="menu"
          aria-label={`Perfil: ${active.name}`}
          className="absolute left-2 top-full z-20 mt-1 w-48 rounded-lg border border-neutral-800 bg-neutral-900 p-1 shadow-xl"
        >
          <button
            type="button"
            role="menuitem"
            className="block w-full rounded px-3 py-2 text-left text-sm text-neutral-200 outline-none hover:bg-neutral-800 focus-visible:bg-neutral-800"
            onClick={() => {
              setOpen(false);
              setProfileId(null);
            }}
          >
            Cambiar de perfil
          </button>
          <Link
            to="/settings"
            role="menuitem"
            className="block w-full rounded px-3 py-2 text-left text-sm text-neutral-200 outline-none hover:bg-neutral-800 focus-visible:bg-neutral-800"
            onClick={() => setOpen(false)}
          >
            Ajustes
          </Link>
        </div>
      )}
    </div>
  );
}
