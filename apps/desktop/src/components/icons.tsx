// Iconos mínimos en SVG inline: evita añadir una librería de iconos solo para media
// docena de trazos. Todos comparten viewBox 24x24 y heredan color/tamaño por CSS.
import type { SVGProps } from "react";

function Icon(props: SVGProps<SVGSVGElement>) {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" {...props} />;
}

export function HomeIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M3 11.5 12 4l9 7.5" />
      <path d="M5 10v9a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1v-9" />
    </Icon>
  );
}

export function LiveIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <rect x="3" y="6" width="18" height="12" rx="2" />
      <path d="M8 21h8M12 18v3" />
    </Icon>
  );
}

export function MovieIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="M3 9h18M8 5v4M16 5v4" />
    </Icon>
  );
}

export function SeriesIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="14" height="10" rx="2" />
      <path d="M8 20h10a2 2 0 0 0 2-2v-8" />
    </Icon>
  );
}

export function SearchIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <circle cx="11" cy="11" r="6" />
      <path d="m20 20-3.5-3.5" />
    </Icon>
  );
}

export function SettingsIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.04 1.56V21a2 2 0 1 1-4 0v-.09A1.7 1.7 0 0 0 9 19.35a1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.65 15a1.7 1.7 0 0 0-1.56-1.04H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.65 1.7 1.7 0 0 0 10.04 3.1V3a2 2 0 1 1 4 0v.09A1.7 1.7 0 0 0 15 4.65a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.35 9a1.7 1.7 0 0 0 1.56 1.04H21a2 2 0 1 1 0 4h-.09A1.7 1.7 0 0 0 19.4 15Z" />
    </Icon>
  );
}

export function ChevronIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="m9 6 6 6-6 6" />
    </Icon>
  );
}

export function HeartIcon(props: SVGProps<SVGSVGElement> & { filled?: boolean }) {
  const { filled, ...rest } = props;
  return (
    <Icon fill={filled ? "currentColor" : "none"} {...rest}>
      <path d="M12 20.5s-7.5-4.6-9.8-9C.6 8 1.8 4.5 5 3.6c2-.6 3.9.2 5 1.8 1.1-1.6 3-2.4 5-1.8 3.2.9 4.4 4.4 2.8 7.9-2.3 4.4-9.8 9-9.8 9Z" />
    </Icon>
  );
}

export function GuideIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 9h18M8 4v5M8 13h4M8 17h8M14 13h6" />
    </Icon>
  );
}

export function PlayIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon fill="currentColor" stroke="none" {...props}>
      <path d="M7 4.5v15l13-7.5-13-7.5Z" />
    </Icon>
  );
}

export function PauseIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon fill="currentColor" stroke="none" {...props}>
      <rect x="6" y="4" width="4" height="16" rx="1" />
      <rect x="14" y="4" width="4" height="16" rx="1" />
    </Icon>
  );
}

export function VolumeIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M4 9v6h4l5 4V5L8 9H4Z" />
      <path d="M17.5 8.5a5 5 0 0 1 0 7" />
    </Icon>
  );
}

export function MuteIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M4 9v6h4l5 4V5L8 9H4Z" />
      <path d="m17 9 4 6M21 9l-4 6" />
    </Icon>
  );
}

export function FullscreenIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M8 3H4v4M16 3h4v4M8 21H4v-4M16 21h4v-4" />
    </Icon>
  );
}

export function ArrowUpIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M12 19V5M5 12l7-7 7 7" />
    </Icon>
  );
}

export function DiceIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <rect x="3" y="3" width="18" height="18" rx="4" />
      <circle cx="8" cy="8" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="16" cy="8" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="8" cy="16" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="16" cy="16" r="1.2" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function ArrowDownIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M12 5v14M19 12l-7 7-7-7" />
    </Icon>
  );
}

export function BellIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <path d="M6 9a6 6 0 1 1 12 0c0 5 2 6.5 2 6.5H4S6 14 6 9Z" />
      <path d="M10 19a2 2 0 0 0 4 0" />
    </Icon>
  );
}

export function MoreIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Icon {...props}>
      <circle cx="5" cy="12" r="1.2" />
      <circle cx="12" cy="12" r="1.2" />
      <circle cx="19" cy="12" r="1.2" />
    </Icon>
  );
}
