import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiDelete, apiGet, apiPost } from "./api-client";
import { useActiveProfile } from "./activeProfile";
import type { ChannelsPage, ReminderResponse } from "./api-types";

type Channel = ChannelsPage["items"][number];
type Programme = { title: string; start_ts: number; stop_ts: number };

/** Cada cuánto se miran los recordatorios: la guía puede llevar abierta horas. */
const REFRESH_MS = 30_000;

/** Recordatorios de programa del perfil activo (F13). Los guarda el núcleo, por perfil. */
export function useReminders() {
  const { profileId } = useActiveProfile();
  const queryClient = useQueryClient();
  const queryKey = ["reminders", profileId];

  const query = useQuery({
    queryKey,
    queryFn: () => apiGet<ReminderResponse[]>(`/api/reminders?profile_id=${profileId}`),
    enabled: profileId !== null,
    refetchInterval: REFRESH_MS,
  });
  const keys = new Set((query.data ?? []).map((r) => `${r.channel.id}:${r.start_ts}`));

  const toggleMutation = useMutation({
    mutationFn: async ({ channel, programme }: { channel: Channel; programme: Programme }) => {
      if (keys.has(`${channel.id}:${programme.start_ts}`)) {
        await apiDelete(`/api/reminders?profile_id=${profileId}&channel_id=${channel.id}&start_ts=${programme.start_ts}`);
      } else {
        await apiPost("/api/reminders", {
          profile_id: profileId,
          channel_id: channel.id,
          start_ts: programme.start_ts,
          stop_ts: programme.stop_ts,
          title: programme.title,
        });
      }
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  });

  const dismissMutation = useMutation({
    mutationFn: (reminder: ReminderResponse) =>
      apiDelete(`/api/reminders?profile_id=${profileId}&channel_id=${reminder.channel.id}&start_ts=${reminder.start_ts}`),
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  });

  return {
    list: query.data ?? [],
    has: (channelId: number, startTs: number) => keys.has(`${channelId}:${startTs}`),
    toggle: (channel: Channel, programme: Programme) => toggleMutation.mutate({ channel, programme }),
    dismiss: (reminder: ReminderResponse) => dismissMutation.mutate(reminder),
  };
}

export type RemindersApi = ReturnType<typeof useReminders>;
