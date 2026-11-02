import { useState } from "react";
import { useApiBase } from "../lib/api-client";
import { channelInitials } from "../lib/channelInitials";

/** El logo de un canal tal como lo encuentra el núcleo (`/logo/<id>`: el del proveedor si
 * funciona; si no, el de la guía, el del directorio abierto o el de un canal hermano, ya
 * guardado en el aparato). Sin ninguno, las iniciales. Ocupa la caja que lo contiene. */
export function ChannelLogoImage({ channelId, name }: { channelId: number; name: string }) {
  const { data: base } = useApiBase();
  const [failed, setFailed] = useState(false);
  if (!base || failed) {
    return <span className="px-0.5 text-center text-[0.7em] font-bold leading-none tracking-tight text-neutral-400">{channelInitials(name)}</span>;
  }
  return <img src={`${base}/logo/${channelId}`} alt="" loading="lazy" onError={() => setFailed(true)} className="h-full w-full object-contain" draggable={false} />;
}
