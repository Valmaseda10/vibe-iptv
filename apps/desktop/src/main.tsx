import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { ActiveProfileProvider } from "./lib/activeProfile";
import { SyncStatusProvider } from "./lib/syncStatus";
import "./styles/globals.css";

// Sin esto (`staleTime` a 0 por defecto de TanStack Query) cada vez que se vuelve a
// montar una pantalla — incluso solo con "Escape" desde una ficha — se dispara una
// petición nueva al servidor local, aunque los datos de hace un segundo sigan siendo
// perfectamente válidos: el catálogo solo cambia de verdad tras una sincronización
// manual o programada, no en cada renavegación. Una consulta que necesite algo más
// fresco (el EPG "ahora", ya con su propio `refetchInterval`; el estado de cuenta,
// invalidado a mano justo cuando cambia de verdad) sigue funcionando igual: estos son
// solo los valores por defecto, cualquier `useQuery` puede pedir los suyos propios.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 60_000,
      gcTime: 10 * 60_000,
    },
  },
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ActiveProfileProvider>
        <SyncStatusProvider>
          <RouterProvider router={router} />
        </SyncStatusProvider>
      </ActiveProfileProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
