import { defineConfig, type Plugin } from "vite";

// The dev server injects component CSS as <style> tags, which index.html's CSP blocks; allow
// inline styles in dev only. The build extracts CSS to files and keeps the strict policy.
const devInlineStyles: Plugin = {
  name: "dev-inline-styles",
  apply: "serve",
  transformIndexHtml: (html) => html.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'"),
};
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

// Served at https://haryx.wecoap.com/platform/. "@win" reads the Windows app's shared code (types,
// formatters, mascot renderer) without modifying it; it becomes a package only if a third consumer appears.
export default defineConfig({
  base: "/platform/",
  plugins: [tailwindcss(), svelte(), devInlineStyles],
  resolve: {
    alias: {
      "@win": fileURLToPath(new URL("../windows/src", import.meta.url)),
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  server: { host: "127.0.0.1", port: 5173, strictPort: true, fs: { allow: [".", "../windows/src", "../../packages"] } },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true, sourcemap: false },
  test: { include: ["src/**/*.test.ts"] },
});
