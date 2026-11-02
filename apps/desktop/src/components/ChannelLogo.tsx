import { ChannelLogoImage } from "./ChannelLogoImage";

/** Logo de un canal en una caja (ver `ChannelLogoImage`): las iniciales si no hay ninguno. */
export function ChannelLogo({ channelId, name, className = "h-5 w-5" }: { channelId: number; name: string; className?: string }) {
  return (
    <span className={`flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-neutral-900 ${className}`}>
      <ChannelLogoImage channelId={channelId} name={name} />
    </span>
  );
}
