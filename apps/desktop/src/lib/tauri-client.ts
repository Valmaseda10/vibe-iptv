import { invoke } from "@tauri-apps/api/core";
import type { HealthStatus } from "./api-types";

/** Puerto del servidor axum local, obtenido del núcleo Rust en el arranque. */
export async function getServerPort(): Promise<number> {
  return invoke<number>("get_server_port");
}

/** Consulta GET /api/health en el servidor local y devuelve el estado del núcleo. */
export async function fetchHealth(port: number): Promise<HealthStatus> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`);
  if (!response.ok) {
    throw new Error(`health check failed: ${response.status}`);
  }
  return response.json();
}
