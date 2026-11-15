import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In development the page comes from Vite and everything under /api,
// including the WebSocket, is forwarded to a server started with
// `cargo run -p fb-server`.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8080", ws: true },
    },
  },
  worker: { format: "es" },
  build: { target: "es2022" },
});
