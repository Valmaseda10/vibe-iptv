import { createRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { rootRoute } from "./__root";
import { fetchHealth, getServerPort } from "../lib/tauri-client";

function HomeScreen() {
  const { data, error, isPending } = useQuery({
    queryKey: ["health"],
    queryFn: async () => {
      const port = await getServerPort();
      const health = await fetchHealth(port);
      return { port, health };
    },
  });

  return (
    <main className="flex h-full flex-col items-center justify-center gap-4">
      <h1 className="text-4xl font-semibold tracking-tight">VIBE IPTV</h1>
      {isPending && <p className="text-neutral-400">Conectando con el núcleo…</p>}
      {error && (
        <p className="text-red-400">
          No se pudo conectar con el núcleo: {error.message}
        </p>
      )}
      {data && (
        <p className="text-lg text-neutral-300">
          {data.health.status === "ok" ? "núcleo ok" : "núcleo con problemas"} · puerto{" "}
          {data.port}
        </p>
      )}
      <Link to="/settings" className="text-red-400 underline hover:text-red-300">
        Ajustes de proveedores
      </Link>
    </main>
  );
}

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomeScreen,
});
