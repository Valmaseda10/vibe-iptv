// Recordar "dónde estaba" al volver a una pantalla (filtros elegidos, scroll de una
// rejilla) — Netflix no te devuelve al principio del catálogo cada vez que abres una
// ficha y pulsas atrás, esta app sí lo hacía porque las pantallas de Películas/Series
// se desmontan al navegar fuera (F "premium").
//
// Vive en memoria, no en localStorage: es estado de UI efímero de esta sesión de la
// app, no algo que deba sobrevivir a un reinicio ni sincronizarse entre perfiles.
const store = new Map<string, unknown>();

export function rememberScreenState<T>(key: string, value: T): void {
  store.set(key, value);
}

export function recallScreenState<T>(key: string): T | undefined {
  return store.get(key) as T | undefined;
}
