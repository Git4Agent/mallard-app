import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // The only consumer is the Tauri webview, so the browser baseline is the one
  // Tauri 2 guarantees — WKWebView on macOS 10.15+, WebView2 on Windows,
  // WebKitGTK 2.24+ — not Vite's default browser matrix. Pinning it stops the
  // build from shipping transpiled fallbacks for engines that never run this.
  build: {
    target: ["es2022", "safari15", "chrome105"],
    // The bundle is packaged into a signed application; sourcemaps would ship
    // the whole frontend inside it and are recoverable from the repo anyway.
    sourcemap: false,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
