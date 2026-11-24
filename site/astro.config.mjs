import { defineConfig } from "astro/config";

// GitHub Pages project site: https://conghai20.github.io/haryx/
// For a custom domain later: set SITE_URL and SITE_BASE="/" in the workflow.
export default defineConfig({
  site: process.env.SITE_URL ?? "https://conghai20.github.io",
  base: process.env.SITE_BASE ?? "/haryx",
  trailingSlash: "ignore",
});
