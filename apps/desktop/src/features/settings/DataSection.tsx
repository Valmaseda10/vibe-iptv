import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { apiGet, errorMessage } from "../../lib/api-client";
import type { BackupSummary, LibraryStats, LogoCoverage, ProfileSummary, SyncProgress } from "../../lib/api-types";
import { applyUiBackup, collectUiBackup } from "../../lib/backupPrefs";
import { SettingsCard } from "../../components/SettingsCard";
import { SettingsRow } from "../../components/SettingsRow";
import { BUTTON, BUTTON_PRIMARY, INPUT } from "./styles";

/** Ajustes → Datos y copia: el catálogo de este aparato, las portadas guardadas, la clave
 * de TMDB y la copia de seguridad (sin contraseñas ni datos de proveedores). */
export function DataSection() {
  const queryClient = useQueryClient();
  const [tmdbApiKey, setTmdbApiKey] = useState("");
  const [tmdbProgress, setTmdbProgress] = useState<SyncProgress | null>(null);
  const [backupMessage, setBackupMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const statsQuery = useQuery({ queryKey: ["library-stats"], queryFn: () => invoke<LibraryStats>("library_stats") });
  const logosQuery = useQuery({ queryKey: ["logo-coverage"], queryFn: () => invoke<LogoCoverage>("logo_coverage") });
  const findLogos = useMutation({
    mutationFn: () => invoke<LogoCoverage>("find_logos"),
    onSuccess: (coverage) => {
      queryClient.setQueryData(["logo-coverage"], coverage);
      // Las listas de canales vuelven a pedir los logos.
      queryClient.invalidateQueries({ queryKey: ["live-channels"] });
    },
  });
  const tmdbKeyQuery = useQuery({ queryKey: ["tmdb-api-key-configured"], queryFn: () => invoke<boolean>("has_tmdb_api_key") });
  const profilesQuery = useQuery({ queryKey: ["profiles"], queryFn: () => apiGet<ProfileSummary[]>("/api/profiles") });

  useEffect(() => {
    const unlisten = listen<SyncProgress>("sync://progress", ({ payload }) => {
      if (payload.stage !== "tmdb") return;
      setTmdbProgress(payload);
      // La clave pudo llegar por la cuenta desde otro aparato: deja de pedirla.
      if (queryClient.getQueryData(["tmdb-api-key-configured"]) !== true) queryClient.invalidateQueries({ queryKey: ["tmdb-api-key-configured"] });
    });
    return () => void unlisten.then((fn) => fn());
  }, [queryClient]);

  const saveKey = useMutation({
    mutationFn: (apiKey: string) => invoke<void>("set_tmdb_api_key", { apiKey }),
    onSuccess: () => {
      setTmdbApiKey("");
      queryClient.invalidateQueries({ queryKey: ["tmdb-api-key-configured"] });
    },
  });
  const matchTmdb = useMutation({ mutationFn: () => invoke<void>("match_tmdb") });
  const clearCache = useMutation({
    mutationFn: () => invoke<number>("clear_image_cache"),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["library-stats"] }),
  });

  const exportBackup = useMutation({
    mutationFn: () => invoke<string>("export_backup", { ui: collectUiBackup(profilesQuery.data ?? []) }),
    onSuccess: (path) => setBackupMessage({ ok: true, text: `Copia guardada en ${path}` }),
    onError: (err) => setBackupMessage({ ok: false, text: errorMessage(err) }),
  });
  const importBackup = useMutation({
    mutationFn: async (file: File) => {
      const contents = await file.text();
      const summary = await invoke<BackupSummary>("import_backup", { contents });
      // Los perfiles pueden ser nuevos: se vuelven a pedir antes de repartir sus preferencias.
      const profiles = await apiGet<ProfileSummary[]>("/api/profiles");
      applyUiBackup(contents, profiles);
      return summary;
    },
    onSuccess: (summary) => {
      setBackupMessage({ ok: true, text: describeImport(summary) });
      queryClient.invalidateQueries();
    },
    onError: (err) => setBackupMessage({ ok: false, text: errorMessage(err) }),
  });

  const stats = statsQuery.data;
  return (
    <div className="flex flex-col gap-6">
      <SettingsCard title="En este aparato">
        <SettingsRow
          title="Catálogo"
          description={
            stats
              ? `${stats.channels.toLocaleString()} canales · ${stats.movies.toLocaleString()} películas · ${stats.series.toLocaleString()} series`
              : "…"
          }
        />
        <SettingsRow
          title="Portadas guardadas"
          description={stats ? `${formatBytes(stats.image_cache_bytes)} en ${stats.image_cache_files.toLocaleString()} imágenes. Se vuelven a descargar según las ves.` : "…"}
        >
          <button type="button" className={BUTTON} onClick={() => clearCache.mutate()} disabled={clearCache.isPending || !stats?.image_cache_files}>
            {clearCache.isPending ? "Vaciando…" : "Vaciar"}
          </button>
        </SettingsRow>
        <SettingsRow
          title="Logos de canales"
          description={
            logosQuery.data
              ? `${logosQuery.data.with_logo} de ${logosQuery.data.total} canales con logo. Si el del proveedor no funciona se busca en la guía, en el directorio abierto de iptv-org y en canales hermanos; se revisa solo cada 12 horas.`
              : "…"
          }
        >
          <button type="button" className={BUTTON} onClick={() => findLogos.mutate()} disabled={findLogos.isPending}>
            {findLogos.isPending ? "Buscando…" : "Buscar ahora"}
          </button>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Copia de seguridad"
        footer="Perfiles, Mi lista, valoraciones, progreso, equipos, recordatorios, subtítulos y preferencias de reproducción. Nunca contraseñas ni datos de tus proveedores. Con cuenta en la nube no hace falta: todo eso ya viaja solo."
      >
        <SettingsRow title="Exportar" description="Guarda un fichero en Documentos para llevarlo a otro aparato.">
          <button type="button" className={BUTTON} onClick={() => exportBackup.mutate()} disabled={exportBackup.isPending}>
            {exportBackup.isPending ? "Guardando…" : "Exportar copia"}
          </button>
        </SettingsRow>
        <SettingsRow title="Importar" description="Añade lo de una copia a lo que ya tienes; si algo está en los dos, gana lo más reciente.">
          <input
            ref={fileInput}
            type="file"
            accept=".json,application/json"
            className="hidden"
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) importBackup.mutate(file);
            }}
          />
          <button type="button" className={BUTTON} onClick={() => fileInput.current?.click()} disabled={importBackup.isPending}>
            {importBackup.isPending ? "Importando…" : "Importar copia"}
          </button>
        </SettingsRow>
        {backupMessage && (
          <p role="status" className={`break-all px-4 py-3 text-[13px] ${backupMessage.ok ? "text-emerald-400" : "text-red-400"}`}>
            {backupMessage.text}
          </p>
        )}
      </SettingsCard>

      <SettingsCard title="Portadas y sinopsis (TMDB)" footer="La clave es gratuita en themoviedb.org. Se guarda cifrada en este aparato y, con cuenta, llega sola a los demás.">
        <SettingsRow
          title="Clave de API"
          description={tmdbKeyQuery.data ? "Guardada." : "Sin clave, las películas y series se quedan sin portada ni sinopsis."}
          stacked
        >
          <div className="flex w-full flex-col gap-3 sm:flex-row">
            <input type="password" placeholder={tmdbKeyQuery.data ? "Cambiar la clave" : "Clave de API de TMDB"} aria-label="Clave de API de TMDB" className={INPUT} value={tmdbApiKey} onChange={(e) => setTmdbApiKey(e.target.value)} />
            <button type="button" className={BUTTON} onClick={() => saveKey.mutate(tmdbApiKey)} disabled={!tmdbApiKey || saveKey.isPending}>
              {saveKey.isPending ? "Guardando…" : "Guardar"}
            </button>
            <button type="button" className={BUTTON_PRIMARY} onClick={() => matchTmdb.mutate()} disabled={!tmdbKeyQuery.data || matchTmdb.isPending}>
              Buscar en TMDB
            </button>
          </div>
        </SettingsRow>
        {matchTmdb.isError && <p className="px-4 pb-3 text-[13px] text-red-400">{errorMessage(matchTmdb.error)}</p>}
        {tmdbProgress && tmdbProgress.total > 0 && tmdbProgress.current < tmdbProgress.total && (
          <div className="flex flex-col gap-1.5 px-4 py-3">
            <p className="text-[13px] text-neutral-400">
              Emparejando con TMDB: {tmdbProgress.current.toLocaleString()} de {tmdbProgress.total.toLocaleString()}
            </p>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
              <div className="h-full rounded-full bg-red-600 transition-[width]" style={{ width: `${Math.min(100, (tmdbProgress.current / tmdbProgress.total) * 100)}%` }} />
            </div>
          </div>
        )}
      </SettingsCard>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function describeImport(summary: BackupSummary): string {
  const parts = [
    [summary.profiles, "perfil", "perfiles"],
    [summary.my_list, "título en Mi lista", "títulos en Mi lista"],
    [summary.ratings, "valoración", "valoraciones"],
    [summary.progress, "progreso", "progresos"],
    [summary.follows, "equipo", "equipos"],
    [summary.reminders, "recordatorio", "recordatorios"],
  ] as const;
  const added = parts.filter(([n]) => n > 0).map(([n, one, many]) => `${n} ${n === 1 ? one : many}`);
  const text = added.length > 0 ? `Importado: ${added.join(", ")}.` : "Importado: ya tenías todo lo de la copia.";
  return summary.not_found > 0 ? `${text} ${summary.not_found} no están en este catálogo y se han saltado.` : text;
}
