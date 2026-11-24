// Build-time lookup of the latest published release. The page must still build
// (and stay correct) when GitHub is unreachable, so every failure returns null
// and the page falls back to linking the Releases list.

const REPO = "conghai20/haryx";
const TIMEOUT_MS = 5000;

export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases`;

export interface Release {
  version: string;
  url: string;
  publishedAt: string;
}

function isRelease(value: unknown): value is { tag_name: string; html_url: string; published_at: string } {
  if (typeof value !== "object" || value === null) return false;
  const r = value as Record<string, unknown>;
  return typeof r.tag_name === "string" && typeof r.html_url === "string" && typeof r.published_at === "string";
}

export async function latestRelease(): Promise<Release | null> {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    if (!res.ok) {
      console.warn(`[site] latest release lookup failed: HTTP ${res.status}`);
      return null;
    }
    const body: unknown = await res.json();
    if (!isRelease(body)) {
      console.warn("[site] latest release lookup returned an unexpected shape");
      return null;
    }
    return {
      version: body.tag_name.replace(/^v/, ""),
      url: body.html_url,
      publishedAt: body.published_at,
    };
  } catch (error) {
    console.warn(`[site] latest release lookup failed: ${String(error)}`);
    return null;
  }
}
