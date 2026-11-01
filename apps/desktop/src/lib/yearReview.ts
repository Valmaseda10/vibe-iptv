/** «Tu año en VIBE» (F17): qué año se enseña y cuándo sale la tarjeta en Inicio. */

/** Del 1 de diciembre al 15 de enero, la tarjeta de Inicio (decidido con el usuario). */
export function yearReviewSeason(now = new Date()): boolean {
  return now.getMonth() === 11 || (now.getMonth() === 0 && now.getDate() <= 15);
}

/** En la primera quincena de enero, el año que acaba de terminar; si no, el actual. */
export function reviewYear(now = new Date()): number {
  return now.getMonth() === 0 && now.getDate() <= 15 ? now.getFullYear() - 1 : now.getFullYear();
}
