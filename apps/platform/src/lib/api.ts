// Same-origin HaryX API client. Errors are mapped to a closed set so screens render states, not strings.
export type ApiErrorKind = "consent_required" | "unauthorized" | "forbidden" | "not_found" | "conflict" | "network" | "server";

export class ApiError extends Error {
  constructor(
    readonly kind: ApiErrorKind,
    readonly status: number,
    readonly code?: string,
  ) {
    super(code ?? kind);
  }
}

export interface ApiDeps {
  baseUrl?: string;
  /** A fresh access token (refreshed if near expiry). */
  getToken: () => Promise<string>;
  /** Called on 401 so the shell can drop to the signed-out state. */
  onUnauthorized?: () => void;
  fetch?: typeof fetch;
}

async function code(res: Response): Promise<string | undefined> {
  try {
    const body = (await res.json()) as { error?: unknown };
    return typeof body.error === "string" ? body.error : undefined;
  } catch {
    return undefined;
  }
}

export async function mapError(res: Response): Promise<ApiError> {
  const c = await code(res);
  if (res.status === 401) return new ApiError("unauthorized", 401, c);
  if (res.status === 403) return new ApiError(c === "consent_required" ? "consent_required" : "forbidden", 403, c);
  if (res.status === 404) return new ApiError("not_found", 404, c);
  if (res.status === 409 || res.status === 412) return new ApiError("conflict", res.status, c);
  return new ApiError("server", res.status, c);
}

export function createApi(deps: ApiDeps) {
  const base = deps.baseUrl ?? "/api/v1";
  const doFetch = deps.fetch ?? fetch;
  return async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
    let res: Response;
    try {
      const headers = new Headers(init.headers);
      headers.set("Authorization", `Bearer ${await deps.getToken()}`);
      if (init.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
      res = await doFetch(base + path, { ...init, headers });
    } catch {
      throw new ApiError("network", 0);
    }
    if (!res.ok) {
      const err = await mapError(res);
      if (err.kind === "unauthorized") deps.onUnauthorized?.();
      throw err;
    }
    return res.status === 204 ? (undefined as T) : ((await res.json()) as T);
  };
}
