import { defineConfig } from "vite";

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2022",
    minify: "esbuild",
    rollupOptions: {
      input: { overlay: "overlay.html" },
    },
  },
});
