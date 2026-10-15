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

/** Error de la API local con su código de estado: hace falta para distinguir, por
 * ejemplo, "la nube no responde" (503) de "contraseña incorrecta" (401). */
export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

/** Texto de un error para enseñarlo al usuario, sin el prefijo "Error: " que añade
 * `String(err)`. */
export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** El núcleo manda mensajes de error pensados para enseñarse tal cual (p. ej.
 * "email rate limit exceeded", "PIN incorrecto") — antes se perdían y solo llegaba el
 * número de estado (`502`, `401`...) a quien mostraba el error, un fallo real
 * encontrado probando la cuenta de sincronización contra el proveedor real. */
async function throwWithBody(path: string, response: Response): Promise<never> {
  const text = await response.text().catch(() => "");
  throw new ApiError(text || `${path} -> ${response.status}`, response.status);
}

export async function apiGet<T>(path: string): Promise<T> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`);
  if (!response.ok) return throwWithBody(path, response);
  return response.json() as Promise<T>;
}

/** Igual que `apiGet`, pero un 204 (sin contenido) da `null` en vez de reventar
 * `response.json()` sobre un cuerpo vacío. Lo usan las consultas "¿hay algo guardado
 * para esto?" (progreso de reproducción, nivel de la cascada). */
export async function apiGetOrNull<T>(path: string): Promise<T | null> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`);
  if (response.status === 204) return null;
  if (!response.ok) return throwWithBody(path, response);
  return response.json() as Promise<T>;
}

/** Genérico en el tipo de vuelta: la mayoría de llamadas ignoran la respuesta (muchas
 * rutas dan `204 sin contenido`), pero algunas sí devuelven algo que hace falta leer
 * (p. ej. el código de emparejamiento de `/api/lan/pairing/start`). Un 204 da
 * `undefined` en vez de reventar `response.json()` sobre un cuerpo vacío. */
export async function apiPost<T = void>(path: string, body: unknown): Promise<T> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) return throwWithBody(path, response);
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export async function apiPatch(path: string, body: unknown): Promise<void> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) return throwWithBody(path, response);
}

export async function apiDelete(path: string): Promise<void> {
  const base = await getApiBase();
  const response = await fetch(`${base}${path}`, { method: "DELETE" });
  if (!response.ok) return throwWithBody(path, response);
}

/** URL de una imagen cacheada por el núcleo (`GET /img/:size/:image_id`), o `null` si
 * no hay ruta de TMDB todavía (p. ej. `tmdb_state` sigue en `pending`). */
export function imagePath(base: string | undefined, path: string | null | undefined, size: "w342" | "w500" | "w1280"): string | null {
  if (!base || !path) return null;
  const clean = path.startsWith("/") ? path.slice(1) : path;
  return `${base}/img/${size}/${clean}`;
}
