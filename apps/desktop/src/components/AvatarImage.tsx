import { imagePath, useApiBase } from "../lib/api-client";
import { backgroundCss, type Avatar } from "../lib/avatar";
import { characterUrl, fluentUrl } from "../lib/avatarImages";

/** Un avatar de perfil, sea del tipo que sea (inicial, emoji, icono 3D, personaje o imagen
 * de una película o serie). El tamaño, el redondeo y el tamaño de letra los pone `className`. */
export function AvatarImage({ avatar, name, className = "h-10 w-10 rounded-md text-lg" }: { avatar: Avatar; name: string; className?: string }) {
  const { data: base } = useApiBase();
  const frame = `relative inline-flex shrink-0 items-center justify-center overflow-hidden font-semibold text-white ${className}`;

  if (avatar.kind === "title") {
    const src = imagePath(base, avatar.path, "w185");
    return (
      <span aria-hidden className={`${frame} bg-neutral-800`}>
        {src && <img src={src} alt="" className="h-full w-full object-cover object-[50%_25%]" draggable={false} />}
      </span>
    );
  }

  const style = { background: backgroundCss(avatar.bg) };
  if (avatar.kind === "fluent") {
    const src = fluentUrl(avatar.slug);
    return (
      <span aria-hidden className={frame} style={style}>
        {src ? <img src={src} alt="" className="h-[78%] w-[78%] object-contain drop-shadow-[0_4px_8px_rgba(0,0,0,0.35)]" draggable={false} /> : name.charAt(0).toUpperCase()}
      </span>
    );
  }
  if (avatar.kind === "character") {
    return (
      <span aria-hidden className={frame} style={style}>
        <img src={characterUrl(avatar.style, avatar.seed)} alt="" className="h-full w-full object-cover" draggable={false} />
      </span>
    );
  }
  return (
    <span aria-hidden className={frame} style={style}>
      {avatar.kind === "emoji" ? avatar.emoji : name.charAt(0).toUpperCase()}
    </span>
  );
}
