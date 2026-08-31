import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { ActiveProfileProvider } from "./lib/activeProfile";
import "./styles/globals.css";

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ActiveProfileProvider>
        <RouterProvider router={router} />
      </ActiveProfileProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
