import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vitejs.dev/config/
// Tauri-aware Vite config: dev server runs on a fixed port, build
// emits to apps/desktop/dist/, which tauri.conf.json's frontendDist
// (../dist relative to src-tauri) consumes.
export default defineConfig({
  plugins: [react()],
  // Vite handles env vars prefixed with VITE_, and we set TAURI_DEBUG
  // here in case any code branches on it.
  envPrefix: ["VITE_", "TAURI_"],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: false,
  },
  build: {
    target: "es2022",
    sourcemap: !!process.env.TAURI_DEBUG,
    minify: !process.env.TAURI_DEBUG,
  },
});
