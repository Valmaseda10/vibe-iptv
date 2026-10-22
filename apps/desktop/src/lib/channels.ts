import type { ChannelsPage } from "./api-types";

type ChannelItem = ChannelsPage["items"][number];

/** Varios paneles Xtream listan el mismo canal varias veces como feeds de respaldo
 * distintos (mismo nombre, `remote_id` distinto — visto de verdad contra un proveedor
 * real, no un caso hipotético). Sin esto, "La 1" aparecía tres veces seguidas sin
 * explicación. Se agrupan por nombre exacto conservando el orden de aparición, no se
 * toca la paginación del backend (agrupar en SQL entre páginas es mucho más frágil que
 * agrupar aquí sobre todo lo ya cargado). */
export type ChannelGroup = {
  displayName: string;
  channels: ChannelItem[];
};

export function groupChannelsByName(items: ChannelItem[]): ChannelGroup[] {
  const byName = new Map<string, ChannelGroup>();
  const order: ChannelGroup[] = [];
  for (const channel of items) {
    let group = byName.get(channel.display_name);
    if (!group) {
      group = { displayName: channel.display_name, channels: [] };
      byName.set(channel.display_name, group);
      order.push(group);
    }
    group.channels.push(channel);
  }
  return order;
}

/** Un canal por nombre: el primer feed de cada grupo. Para la guía, donde todos los feeds
 * de un mismo canal tienen la misma programación y repetir la fila solo estorba. */
export function uniqueChannelsByName(items: ChannelItem[]): ChannelItem[] {
  return groupChannelsByName(items).map((group) => group.channels[0]).filter((c): c is ChannelItem => c !== undefined);
}

/** Nombres de las fuentes de un mismo canal para el selector: su calidad e idioma («4K»,
 * «HD · ES») o «Fuente N» si el nombre no traía ninguno. Si dos coinciden, se numeran. */
export function feedLabels(channels: ChannelItem[]): string[] {
  const base = channels.map((channel, index) => {
    const details = [channel.quality, channel.language].filter((part): part is string => Boolean(part));
    return details.length > 0 ? details.join(" · ") : `Fuente ${index + 1}`;
  });
  const seen = new Map<string, number>();
  const total = new Map<string, number>();
  for (const label of base) total.set(label, (total.get(label) ?? 0) + 1);
  return base.map((label) => {
    if ((total.get(label) ?? 0) < 2) return label;
    const n = (seen.get(label) ?? 0) + 1;
    seen.set(label, n);
    return `${label} (${n})`;
  });
}
