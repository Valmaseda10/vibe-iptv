/** La app corre en un iPhone o iPad (F15). Se decide al cargar, sin preguntar al núcleo:
 * el reproductor y la disposición cambian desde el primer render. El iPad con iPadOS se
 * presenta como "Macintosh", pero con pantalla táctil (ningún Mac la tiene). */
export const IS_IOS: boolean = (() => {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent;
  return /iPhone|iPad|iPod/.test(ua) || (ua.includes("Macintosh") && navigator.maxTouchPoints > 1);
})();
