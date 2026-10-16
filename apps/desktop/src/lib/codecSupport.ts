/** Audio que la WebView de este equipo decodifica por sí misma, en el formato que
 * espera `?audio_passthrough=` del núcleo ("ac3,eac3"). La WebView2 de Windows reproduce
 * Dolby AC-3/E-AC-3 (comprobado midiendo la señal, no solo con `canPlayType`): pasarlo
 * tal cual conserva el 5.1 en vez de convertirlo a AAC estéreo. Otras WebViews (Linux,
 * versiones antiguas) dicen que no y el núcleo sigue convirtiendo como siempre. */
export const AUDIO_PASSTHROUGH: string = (() => {
  if (typeof document === "undefined") return "";
  const video = document.createElement("video");
  const supported: string[] = [];
  if (video.canPlayType('audio/mp4; codecs="ac-3"') !== "") supported.push("ac3");
  if (video.canPlayType('audio/mp4; codecs="ec-3"') !== "") supported.push("eac3");
  return supported.join(",");
})();
