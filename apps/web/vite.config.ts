import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The dev server proxies to the Rust API so the browser sees one origin and no CORS configuration is needed. In production the API serves these built files itself.
export default defineConfig({
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: Number(process.env.WEB_PORT ?? 5173),
    proxy: {
      "/api": { target: `http://127.0.0.1:${process.env.API_PORT ?? 8787}`, changeOrigin: false },
      "/health": { target: `http://127.0.0.1:${process.env.API_PORT ?? 8787}`, changeOrigin: false },
      "/ready": { target: `http://127.0.0.1:${process.env.API_PORT ?? 8787}`, changeOrigin: false },
    },
  }
});
