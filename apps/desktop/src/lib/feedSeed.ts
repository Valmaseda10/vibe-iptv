/** Semilla de las filas de Inicio, Películas y Series (`GET /api/browse`). Cambia al abrir
 * la app, tras 20 minutos sin pasar por estas pantallas y, con la app abierta todo el día,
 * cada dos horas: entrar dos o tres veces el mismo día no enseña lo mismo, pero ir a una
 * ficha y volver no baraja la pantalla. */
const IDLE_LIFETIME_MS = 20 * 60_000;
const MAX_LIFETIME_MS = 2 * 60 * 60_000;
const RECENT_FEATURED_KEY = "vibe.browse.recentFeatured";
const RECENT_FEATURED_MAX = 6;

type Visit = { seed: number; avoid: string; createdAt: number; usedAt: number };

let current: Visit | null = null;

function newVisit(now: number): Visit {
  // Las portadas a evitar se fijan al empezar la visita: si no, volver de una ficha
  // cambiaría la portada que se acaba de ver.
  return { seed: Math.floor(Math.random() * 2 ** 48), avoid: recentFeatured().join(","), createdAt: now, usedAt: now };
}

/** La visita actual: semilla y portadas de visitas anteriores que no repetir (renueva una
 * visita caducada). */
export function feedVisit(): { seed: number; avoid: string } {
  const now = Date.now();
  if (!current || now - current.usedAt > IDLE_LIFETIME_MS || now - current.createdAt > MAX_LIFETIME_MS) {
    current = newVisit(now);
  } else {
    current.usedAt = now;
  }
  return { seed: current.seed, avoid: current.avoid };
}

/** Portadas de las últimas visitas (`movie:12`), para no repetirlas. Solo en este aparato. */
export function recentFeatured(): string[] {
  try {
    const raw = window.localStorage.getItem(RECENT_FEATURED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

export function rememberFeatured(kind: string, id: number): void {
  const key = `${kind}:${id}`;
  const next = [key, ...recentFeatured().filter((item) => item !== key)].slice(0, RECENT_FEATURED_MAX);
  try {
    window.localStorage.setItem(RECENT_FEATURED_KEY, JSON.stringify(next));
  } catch {
    // Sin almacenamiento: solo puede repetirse la portada.
  }
}
