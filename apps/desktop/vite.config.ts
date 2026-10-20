import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri espera que Vite escuche en un puerto fijo conocido por tauri.conf.json,
// y que ignore los cambios de src-tauri para no reiniciar el HMR al compilar Rust.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Número de compilación visible en Ajustes: el flujo de iOS pasa el número de
  // ejecución de GitHub; en el PC es "local". Todas las versiones dicen 0.1.0 y no había
  // forma de saber cuál tenía instalada el iPhone.
  define: {
    __BUILD_ID__: JSON.stringify(process.env.VIBE_BUILD_ID ?? "local"),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
