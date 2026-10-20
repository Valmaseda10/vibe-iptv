import { useEffect, useRef, useState } from "react";
import { IS_IOS } from "../lib/platform";

/** Título de pantalla. En el PC, como siempre. En iOS (F15), el título grande de iOS
 * (34 pt, negrita) y, cuando se desplaza fuera de la vista, una barra compacta de cristal
 * arriba con el mismo título en pequeño, como en las apps del sistema. Las pantallas que
 * desplazan una lista propia (Películas, Directo…) no lo mueven: ahí nunca se compacta. */
export function LargeTitle({ children, className = "" }: { children: string; className?: string }) {
  const ref = useRef<HTMLHeadingElement>(null);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    const node = ref.current;
    if (!IS_IOS || !node) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[0];
        if (entry) setCollapsed(!entry.isIntersecting && entry.boundingClientRect.top < 0);
      },
      { rootMargin: "-40px 0px 0px 0px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  return (
    <>
      <h1
        ref={ref}
        className={`${IS_IOS ? "text-[34px] font-bold leading-[41px] tracking-tight" : "text-2xl font-semibold sm:text-3xl"} ${className}`}
      >
        {children}
      </h1>
      {IS_IOS && (
        <div
          aria-hidden="true"
          className={`glass fixed inset-x-0 top-0 z-40 flex h-[calc(44px+env(safe-area-inset-top))] items-end justify-center rounded-none border-x-0 border-t-0 pb-2.5 shadow-none transition-opacity duration-200 desk:hidden ${
            collapsed ? "opacity-100" : "pointer-events-none opacity-0"
          }`}
        >
          <span className="text-[17px] font-semibold text-white">{children}</span>
        </div>
      )}
    </>
  );
}
