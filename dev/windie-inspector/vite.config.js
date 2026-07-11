import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    host: "127.0.0.1",
    port: 3000,
    strictPort: true,
    headers: {
      "Cross-Origin-Resource-Policy": "same-origin",
    },
  },
  build: {
    outDir: "build",
  },
});
