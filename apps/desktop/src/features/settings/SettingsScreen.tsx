import { useMemo, useState, type ComponentType, type SVGProps } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { LargeTitle } from "../../components/LargeTitle";
import { BellIcon, ChevronIcon, DatabaseIcon, InfoIcon, PlayIcon, SearchIcon, ServerIcon, ShieldIcon, SparkIcon, UserIcon } from "../../components/icons";
import { AccountSection } from "./AccountSection";
import { ProvidersSection } from "./ProvidersSection";
import { PlaybackSection } from "./PlaybackSection";
import { TasteSection } from "./TasteSection";
import { NotificationsSection } from "./NotificationsSection";
import { ParentalSection } from "./ParentalSection";
import { DataSection } from "./DataSection";
import { AboutSection } from "./AboutSection";
import { BinariesStatusBanner } from "./BinariesStatusBanner";
import { SettingsProfileHeader } from "./SettingsProfileHeader";

type Section = {
  id: string;
  label: string;
  description: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
  /** Color del cuadradito del icono, como en los Ajustes de iOS. */
  tint: string;
  /** Palabras con las que el buscador encuentra la sección. */
  keywords: string;
  component: ComponentType;
};

const SECTIONS: Section[] = [
  {
    id: "cuenta",
    label: "Cuenta y perfiles",
    description: "Tu perfil, la cuenta en la nube y la contraseña.",
    icon: UserIcon,
    tint: "bg-sky-600",
    keywords: "cuenta correo contraseña sesión nube perfil perfiles pin cerrar sesión",
    component: AccountSection,
  },
  {
    id: "proveedores",
    label: "Proveedores",
    description: "Tus listas IPTV: estado, caducidad y sincronización.",
    icon: ServerIcon,
    tint: "bg-red-600",
    keywords: "proveedor lista xtream m3u sincronizar caduca conexiones añadir eliminar guía epg",
    component: ProvidersSection,
  },
  {
    id: "reproduccion",
    label: "Reproducción",
    description: "Siguiente episodio, idioma de audio y subtítulos.",
    icon: PlayIcon,
    tint: "bg-violet-600",
    keywords: "reproducción reproductor subtítulos audio idioma siguiente episodio reanudar cuenta atrás",
    component: PlaybackSection,
  },
  {
    id: "gustos",
    label: "Tus gustos",
    description: "Lo que has valorado, que guía las recomendaciones.",
    icon: SparkIcon,
    tint: "bg-pink-600",
    keywords: "gustos me gusta valoraciones recomendaciones",
    component: TasteSection,
  },
  {
    id: "avisos",
    label: "Avisos",
    description: "Episodios nuevos, recordatorios y partidos.",
    icon: BellIcon,
    tint: "bg-amber-500",
    keywords: "avisos notificaciones episodios nuevos recordatorios partidos",
    component: NotificationsSection,
  },
  {
    id: "control-parental",
    label: "Control parental",
    description: "Qué categorías no ven los perfiles infantiles.",
    icon: ShieldIcon,
    tint: "bg-emerald-600",
    keywords: "control parental adultos categorías niños infantil pin",
    component: ParentalSection,
  },
  {
    id: "datos",
    label: "Datos y copia",
    description: "Catálogo, portadas, TMDB y copia de seguridad.",
    icon: DatabaseIcon,
    tint: "bg-cyan-600",
    keywords: "datos tmdb clave caché portadas imágenes copia de seguridad exportar importar catálogo",
    component: DataSection,
  },
  {
    id: "acerca",
    label: "Acerca de",
    description: "Versión, actualizaciones y novedades.",
    icon: InfoIcon,
    tint: "bg-neutral-500",
    keywords: "acerca versión actualizaciones actualizar novedades diagnóstico registro compilación",
    component: AboutSection,
  },
];

function matches(section: Section, needle: string): boolean {
  const text = `${section.label} ${section.keywords}`.toLowerCase();
  return needle.split(/\s+/).every((word) => text.includes(word));
}

/** Ajustes (rediseño 2026-11-02): en el PC y el iPad, las secciones a la izquierda y su
 * contenido a la derecha, como en Windows 11; en el iPhone, una lista agrupada como la de
 * iOS en la que cada fila abre su pantalla. Con buscador en los dos. */
export function SettingsScreen({ section }: { section?: string }) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const visible = useMemo(() => (needle ? SECTIONS.filter((s) => matches(s, needle)) : SECTIONS), [needle]);
  const current = SECTIONS.find((s) => s.id === section);
  // En el PC, sin sección elegida se ve la primera; en el iPhone, la lista.
  const shown = current ?? (SECTIONS[0] as Section);

  function openFirstMatch(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key !== "Enter" || !visible[0]) return;
    navigate({ to: "/settings/$section", params: { section: visible[0].id } });
  }

  return (
    <div className="mx-auto flex w-full max-w-6xl gap-8 p-4 sm:p-8 desk:items-start">
      {/* Lista de secciones: columna izquierda en el PC; la pantalla entera en el iPhone. */}
      <nav aria-label="Secciones de ajustes" className={`w-full flex-col gap-4 desk:sticky desk:top-8 desk:flex desk:w-64 desk:shrink-0 ${current ? "hidden" : "flex"}`}>
        <LargeTitle>Ajustes</LargeTitle>
        <SettingsProfileHeader />
        <label className="relative block">
          <span className="sr-only">Buscar en ajustes</span>
          <SearchIcon className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-neutral-500" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={openFirstMatch}
            placeholder="Buscar en ajustes"
            className="w-full rounded-xl bg-white/[0.06] py-2.5 pl-9 pr-3 text-sm text-white outline-none ring-1 ring-white/[0.06] placeholder:text-neutral-500 focus:ring-white/30 ios:rounded-[12px] ios:bg-ios-surface-1 ios:ring-0"
          />
        </label>
        <ul className="flex flex-col overflow-hidden rounded-xl bg-white/[0.03] ring-1 ring-white/[0.06] desk:bg-transparent desk:ring-0 ios:rounded-[18px] ios:bg-ios-surface-1 ios:ring-0 desk:ios:bg-transparent">
          {visible.map((s) => {
            const Icon = s.icon;
            const active = shown.id === s.id;
            return (
              <li key={s.id} className="border-b border-white/[0.05] last:border-0 desk:border-0">
                <Link
                  to="/settings/$section"
                  params={{ section: s.id }}
                  aria-current={active ? "page" : undefined}
                  className={`flex items-center gap-3 px-3 py-2.5 text-[15px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-white/70 desk:rounded-lg ${
                    active ? "desk:bg-white/10 desk:text-white" : "text-neutral-200 hover:bg-white/[0.05]"
                  }`}
                >
                  <span className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] text-white ${s.tint}`}>
                    <Icon className="h-4 w-4" />
                  </span>
                  <span className="min-w-0 flex-1 truncate">{s.label}</span>
                  <ChevronIcon className="h-4 w-4 text-neutral-600 desk:hidden" />
                </Link>
              </li>
            );
          })}
          {visible.length === 0 && <li className="px-3 py-3 text-sm text-neutral-500">Nada coincide con «{query}».</li>}
        </ul>
      </nav>

      {/* Contenido de la sección: a la derecha en el PC; pantalla propia en el iPhone. */}
      <section className={`min-w-0 flex-1 flex-col gap-6 desk:flex ${current ? "flex" : "hidden"}`} aria-labelledby="settings-section-title">
        <Link to="/settings" className="flex w-fit items-center gap-1 text-[15px] text-red-500 outline-none desk:hidden ios:text-[#0a84ff]">
          <ChevronIcon className="h-4 w-4 rotate-180" />
          Ajustes
        </Link>
        <header className="flex flex-col gap-1">
          <h1 id="settings-section-title" className="text-2xl font-bold text-white sm:text-3xl ios:text-[30px] ios:tracking-tight">
            {shown.label}
          </h1>
          <p className="text-sm text-neutral-400 ios:text-ios-label-2">{shown.description}</p>
        </header>
        <BinariesStatusBanner />
        <shown.component key={shown.id} />
      </section>
    </div>
  );
}
