import { useQuery } from "@tanstack/react-query";
import { apiGet } from "../../lib/api-client";
import type { BinariesStatus } from "../../lib/api-types";
import { IS_IOS } from "../../lib/platform";

/** Comprobación de que existen ffmpeg, ffprobe y mpv (F10): no debería faltar ninguno con el
 * instalador, pero puede pasar en desarrollo o con una instalación rota. No se enseña nada
 * mientras todo está bien. En iPhone y iPad no existen: reproduce VLC (F15). */
export function BinariesStatusBanner() {
  const { data } = useQuery({
    queryKey: ["binaries-status"],
    queryFn: () => apiGet<BinariesStatus>("/api/playback/binaries"),
    enabled: !IS_IOS,
  });

  if (IS_IOS || !data) return null;
  const missing = (["ffmpeg", "ffprobe", "mpv"] as const).filter((name) => !data[name]);
  if (missing.length === 0) return null;

  // ffmpeg/ffprobe sostienen casi toda la reproducción (directo, MKV); mpv solo el respaldo.
  const missingCore = !data.ffmpeg || !data.ffprobe;
  return (
    <div className="rounded-xl bg-amber-950/40 p-4 text-sm text-amber-200 ring-1 ring-amber-700">
      Falta{missing.length > 1 ? "n" : ""} <strong>{missing.join(", ")}</strong>:{" "}
      {missingCore
        ? "no se podrá ver el directo ni la mayoría de películas y series."
        : "los contenidos con códecs poco habituales (H.265 sin aceleración, VC-1…) no se podrán abrir en mpv; el resto se ve con normalidad."}{" "}
      Si instalaste la app con el instalador, reinstálala.
    </div>
  );
}
