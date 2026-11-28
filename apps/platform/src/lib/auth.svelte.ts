// keycloak-js wrapper. Tokens live in memory only (keycloak-js keeps them on its instance; nothing
// is written to storage). Flow: PKCE S256 and a silent check-sso: a hidden iframe asks WeCoAp SSO
// (prompt=none) whether this browser is already signed in and hands the answer back through
// public/silent-check-sso.html, so the page never leaves. Signed in: straight to the console.
// Not signed in: the sign-in screen (SignedOut). When the browser blocks the cookie check,
// keycloak-js falls back to the same check by redirect. No session-status iframe is kept open.
import Keycloak from "keycloak-js";
import { config } from "./config";
import { createApi } from "./api";
import { startRefresh } from "./refresh";
import { withTimeout } from "./timeout";

export type AuthStatus = "starting" | "signedOut" | "signedIn" | "failed";

/** How long the silent check may take before the page offers "Try again" instead of waiting on. */
export const SSO_CHECK_TIMEOUT_MS = 15_000;

class Auth {
  status = $state<AuthStatus>("starting");
  email = $state<string | null>(null);
  private kc = new Keycloak({ url: config.url, realm: config.realm, clientId: config.clientId });
  private stop: (() => void) | null = null;

  async init() {
    try {
      // A silent check whose iframe never answers (SSO down, redirect URI refused) would otherwise
      // leave the start screen up for good.
      const ok = await withTimeout(this.kc.init({
        onLoad: "check-sso",
        pkceMethod: "S256",
        checkLoginIframe: false,
        silentCheckSsoRedirectUri: new URL("silent-check-sso.html", location.origin + import.meta.env.BASE_URL).href,
      }), SSO_CHECK_TIMEOUT_MS);
      if (ok) this.enter();
      else this.status = "signedOut";
    } catch {
      this.status = "failed";
    }
  }

  private enter() {
    this.email = (this.kc.tokenParsed?.email as string | undefined) ?? null;
    this.status = "signedIn";
    this.stop?.();
    this.stop = startRefresh(this.kc.tokenParsed?.exp ?? 0, async () => {
      try {
        await this.kc.updateToken(-1);
        return this.kc.tokenParsed?.exp ?? null;
      } catch {
        this.drop();
        return null;
      }
    });
  }

  /** Session id of this browser session (`sid` claim), for the machine "this is you" check. */
  get sid(): string | undefined {
    return this.kc.tokenParsed?.sid as string | undefined;
  }

  /** A token valid for at least 30 s; ends the session if refresh fails. */
  async token(): Promise<string> {
    try {
      await this.kc.updateToken(30);
    } catch {
      this.drop();
      throw new Error("session expired");
    }
    if (!this.kc.token) throw new Error("no token");
    return this.kc.token;
  }

  signIn = () => this.kc.login({ redirectUri: location.origin + location.pathname });
  signOut = () => this.kc.logout({ redirectUri: location.origin + location.pathname });
  drop = () => {
    this.stop?.();
    this.email = null;
    this.status = "signedOut";
  };
}

export const auth = new Auth();
export const api = createApi({ getToken: () => auth.token(), onUnauthorized: () => auth.drop() });
