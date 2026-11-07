// Generado por tools/publicar-version.mjs a partir de las notas de la versión: no se edita a mano.
export const RELEASE_NOTES: { version: string; notes: readonly string[] } = {
  "version": "0.3.5",
  "notes": [
    "Directo: ya no se para cuando el proveedor pierde trozos del canal (pasa a menudo con una VPN). La imagen sigue, el sonido no se corta y la app no vuelve a conectar con el proveedor cada pocos segundos.",
    "Directo: si el canal llega algo lento, se reproduce un poco más despacio sin que se note, en vez de pararse cada poco.",
    "Añadir proveedor: si el proveedor rechaza la conexión (error 403), la app explica por qué suele pasar. Y si pegas un enlace de lista de un panel Xtream, te ofrece añadirlo como Xtream.",
    "Deportes: si no hay partidos porque la guía no se ha podido actualizar, ahora lo dice."
  ]
};
