import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vitejs.dev/config/
// Tauri-aware Vite config: the build
// emits to apps/desktop/dist/, which tauri.conf.json's frontendDist
// (../dist relative to src-tauri) consumes.
export default defineConfig({
  plugins: [react()],
  // Relative, so the same build loads from Tauri's asset protocol, from
  // `vite preview` at `/`, and from the host under `/p/<project>/`.
  base: "./",
  // Vite handles env vars prefixed with VITE_, and we set TAURI_DEBUG
  // here in case any code branches on it.
  envPrefix: ["VITE_", "TAURI_"],
  clearScreen: false,
  build: {
    target: "es2022",
    sourcemap: !!process.env.TAURI_DEBUG,
    minify: !process.env.TAURI_DEBUG,
  },
});
