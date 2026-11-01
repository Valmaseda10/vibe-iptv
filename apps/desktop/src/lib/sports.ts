import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost } from "./api-client";
import { useActiveProfile } from "./activeProfile";
import { ensureNotificationPermission } from "./notifications";
import type { SportEventResponse, SportFollow } from "./api-types";

/** Deportes (F17): la guía de partidos, el de la portada y «Mis equipos» del perfil. El
 * núcleo lo saca de la guía ya guardada (`catalog::sports`); aquí solo se pide y se pinta. */

const REFRESH_MS = 60_000;

export function useSportsEvents(days = 7) {
  const { profileId } = useActiveProfile();
  return useQuery({
    queryKey: ["sports", profileId, days],
    queryFn: () => apiGet<SportEventResponse[]>(`/api/sports?profile_id=${profileId}&days=${days}`),
    enabled: profileId !== null,
    refetchInterval: REFRESH_MS,
  });
}

export function useFeaturedMatch() {
  const { profileId } = useActiveProfile();
  return useQuery({
    queryKey: ["sports-featured", profileId],
    queryFn: () => apiGet<SportEventResponse | null>(`/api/sports/featured?profile_id=${profileId}`),
    enabled: profileId !== null,
    refetchInterval: REFRESH_MS,
  });
}

/** «Mis equipos»: lo que se puede seguir, con lo seguido marcado, y cómo cambiarlo. */
export function useFollowables() {
  const { profileId } = useActiveProfile();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["sports-followables", profileId],
    queryFn: () => apiGet<SportFollow[]>(`/api/sports/followables?profile_id=${profileId}`),
    enabled: profileId !== null,
  });
  const toggle = useMutation({
    mutationFn: async (item: SportFollow) => {
      if (item.followed) {
        await apiDelete(`/api/sports/follows?profile_id=${profileId}&kind=${item.kind}&key=${encodeURIComponent(item.key)}`);
      } else {
        await apiPost("/api/sports/follows", { profile_id: profileId, kind: item.kind, key: item.key, label: item.label });
        // Seguir algo es pedir avisos de sus partidos: el permiso, ahora.
        void ensureNotificationPermission();
      }
    },
    onMutate: async (item) => {
      const key = ["sports-followables", profileId];
      await queryClient.cancelQueries({ queryKey: key });
      queryClient.setQueryData<SportFollow[]>(key, (old) => old?.map((f) => (f.kind === item.kind && f.key === item.key ? { ...f, followed: !item.followed } : f)));
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["sports-followables"] });
      queryClient.invalidateQueries({ queryKey: ["sports"] });
      queryClient.invalidateQueries({ queryKey: ["sports-featured"] });
    },
  });
  return { list: query.data ?? [], isPending: query.isPending, toggle: (item: SportFollow) => toggle.mutate(item) };
}

export const SPORT_LABELS: Record<string, string> = {
  futbol: "Fútbol",
  "futbol-sala": "Fútbol sala",
  baloncesto: "Baloncesto",
  tenis: "Tenis",
  motor: "Motor",
  balonmano: "Balonmano",
  ciclismo: "Ciclismo",
  golf: "Golf",
  combate: "Boxeo y lucha",
  rugby: "Rugby",
  beisbol: "Béisbol",
  "futbol-americano": "Fútbol americano",
};

const SPORT_EMOJI: Record<string, string> = {
  futbol: "⚽",
  "futbol-sala": "⚽",
  baloncesto: "🏀",
  tenis: "🎾",
  motor: "🏁",
  balonmano: "🤾",
  ciclismo: "🚴",
  golf: "⛳",
  combate: "🥊",
  rugby: "🏉",
  beisbol: "⚾",
  "futbol-americano": "🏈",
};

export function sportEmoji(sport: string | null): string {
  return (sport && SPORT_EMOJI[sport]) || "🏆";
}

/** Si está en juego ahora. */
export function isOnNow(event: SportEventResponse, now = Date.now() / 1000): boolean {
  return event.start_ts <= now && now < event.stop_ts;
}

function startOfDay(epoch: number): number {
  const date = new Date(epoch * 1000);
  date.setHours(0, 0, 0, 0);
  return date.getTime() / 1000;
}

/** «Hoy», «Mañana» o «sábado 8». */
export function dayLabel(epoch: number, now = Date.now() / 1000): string {
  const days = Math.round((startOfDay(epoch) - startOfDay(now)) / 86_400);
  if (days <= 0) return "Hoy";
  if (days === 1) return "Mañana";
  return new Date(epoch * 1000).toLocaleDateString("es-ES", { weekday: "long", day: "numeric" });
}

export function timeLabel(epoch: number): string {
  return new Date(epoch * 1000).toLocaleTimeString("es-ES", { hour: "2-digit", minute: "2-digit" });
}

/** «Empieza en 25 min», «Hoy · 21:00», «EN DIRECTO» (ya empezado). */
export function whenLabel(event: SportEventResponse, now = Date.now() / 1000): string {
  if (isOnNow(event, now)) return "En directo";
  const minutes = Math.round((event.start_ts - now) / 60);
  if (minutes <= 60) return `Empieza en ${Math.max(1, minutes)} min`;
  return `${dayLabel(event.start_ts, now)} · ${timeLabel(event.start_ts)}`;
}

/** Iniciales de un equipo para su escudo dibujado («Real Madrid» → «RM»). */
export function crestInitials(name: string): string {
  const words = name
    .replace(/\(.*?\)/g, "")
    .split(/\s+/)
    .filter((w) => w.length > 2 || /^[A-Z]{2,3}$/.test(w));
  const picked = (words.length > 0 ? words : name.split(/\s+/)).slice(0, 2);
  return picked.map((w) => w[0]?.toUpperCase() ?? "").join("") || "?";
}

/** Color estable de un equipo para su escudo (los conocidos, con sus colores). */
export function crestColor(key: string): string {
  const known: Record<string, string> = {
    "real-madrid": "#e8e8e8",
    barcelona: "#a50044",
    atletico: "#cb3524",
    athletic: "#ee2523",
    "real-sociedad": "#0067b1",
    betis: "#0bb363",
    sevilla: "#d81e05",
    valencia: "#ee8707",
    villarreal: "#ffe667",
    espana: "#c60b1e",
  };
  if (known[key]) return known[key];
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return `hsl(${Math.abs(hash) % 360} 55% 42%)`;
}
