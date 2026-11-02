import { useId } from "react";

/** El logo (2026-11-02): un play con degradado naranja → rojo → magenta y la «V» tallada
 * dentro. Mismo dibujo que el icono de la app (`src-tauri/icons/source/icon.svg`), sin el
 * fondo, para la barra lateral, la pantalla de carga y la de cuenta. */
export function BrandMark({ className = "h-6 w-6" }: { className?: string }) {
  const id = useId();
  return (
    <svg viewBox="290 228 520 568" className={className} aria-hidden="true">
      <defs>
        <linearGradient id={`${id}g`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#ff8a3d" />
          <stop offset="0.5" stopColor="#ff2d55" />
          <stop offset="1" stopColor="#c2187a" />
        </linearGradient>
        <mask id={`${id}m`}>
          <rect x="0" y="0" width="1024" height="1024" fill="#fff" />
          <path d="M384 436 L466 596 L548 436" fill="none" stroke="#000" strokeWidth="72" strokeLinecap="round" strokeLinejoin="round" />
        </mask>
      </defs>
      <path
        d="M312 296 L312 728 Q312 790 366 760 L748 550 Q800 520 748 490 L366 264 Q312 234 312 296 Z"
        fill={`url(#${id}g)`}
        mask={`url(#${id}m)`}
      />
    </svg>
  );
}

/** Logo y nombre juntos. */
export function BrandLockup({ size = "sm" }: { size?: "sm" | "lg" }) {
  const big = size === "lg";
  return (
    <span className={`flex items-center ${big ? "gap-3" : "gap-2"}`}>
      <BrandMark className={big ? "h-11 w-11" : "h-6 w-6"} />
      <span className={`font-extrabold tracking-tight text-white ${big ? "text-4xl" : "text-[17px]"}`}>
        VIBE<span className={`ml-1.5 font-semibold text-white/55 ${big ? "text-2xl" : "text-[12px] tracking-[0.18em]"}`}>IPTV</span>
      </span>
    </span>
  );
}
