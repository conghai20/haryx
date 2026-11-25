import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port in dev and fails if it is taken. The dev server may also read the shared
// mascot sprites outside this folder.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
    fs: { allow: [".", "../../packages/mascots"] },
  },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
  test: { include: ["src/**/*.test.ts"] },
});
