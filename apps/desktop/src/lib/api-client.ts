import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";

let cachedBase: Promise<string> | null = null;

/** Resuelve la URL base del servidor local una sola vez por sesión de la app. */
export function getApiBase(): Promise<string> {
  cachedBase ??= invoke<number>("get_server_port").then((port) => `http://127.0.0.1:${port}`);
  return cachedBase;
}

/** Para componentes: la base ya resuelta, cacheada por TanStack Query. */
export function useApiBase() {
  return useQuery({ queryKey: ["server-base-url"], queryFn: getApiBase, staleTime: Infinity });
}

export async function apiGet<T>(path: string): Promise<T> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`);
  if (!response.ok) throw new Error(`${path} -> ${response.status}`);
  return response.json() as Promise<T>;
}

/** Igual que `apiGet`, pero un 204 (sin contenido) da `null` en vez de reventar
 * `response.json()` sobre un cuerpo vacío. Lo usan las consultas "¿hay algo guardado
 * para esto?" (progreso de reproducción, nivel de la cascada). */
export async function apiGetOrNull<T>(path: string): Promise<T | null> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`);
  if (response.status === 204) return null;
  if (!response.ok) throw new Error(`${path} -> ${response.status}`);
  return response.json() as Promise<T>;
}

export async function apiPost(path: string, body: unknown): Promise<void> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${path} -> ${response.status}`);
}

export async function apiDelete(path: string): Promise<void> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`, { method: "DELETE" });
  if (!response.ok) throw new Error(`${path} -> ${response.status}`);
}

/** URL de una imagen cacheada por el núcleo (`GET /img/:size/:image_id`), o `null` si
 * no hay ruta de TMDB todavía (p. ej. `tmdb_state` sigue en `pending`). */
export function imagePath(base: string | undefined, path: string | null | undefined, size: "w342" | "w500" | "w1280"): string | null {
  if (!base || !path) return null;
  const clean = path.startsWith("/") ? path.slice(1) : path;
  return `${base}/img/${size}/${clean}`;
}
