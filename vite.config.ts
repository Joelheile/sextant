import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  root: "ui",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "ui") } },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "../dist", emptyOutDir: true, target: "safari17" },
});
