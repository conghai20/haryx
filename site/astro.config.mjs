import { defineConfig } from "astro/config";

// Public address https://haryx.wecoap.com, served by a proxy in front of GitHub Pages:
// haryx.wecoap.com/<path> → conghai20.github.io/haryx/<path>. Pages has no custom domain set (it would
// redirect the proxy's fetches back to the proxy), so the site is built for the proxy's root.
export default defineConfig({
  site: process.env.SITE_URL ?? "https://haryx.wecoap.com",
  base: process.env.SITE_BASE ?? "/",
  trailingSlash: "ignore",
});
