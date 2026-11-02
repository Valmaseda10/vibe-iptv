import { useState } from "react";

/** Logo de un canal (del proveedor): si no hay o no carga, «TV» en vez de la imagen rota. */
export function ChannelLogo({ url, className = "h-5 w-5" }: { url: string | null; className?: string }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className={`flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-neutral-900 ${className}`}>
      {url && !failed ? <img src={url} alt="" onError={() => setFailed(true)} className="h-full w-full object-contain" /> : <span className="text-[9px] text-neutral-400">TV</span>}
    </span>
  );
}
