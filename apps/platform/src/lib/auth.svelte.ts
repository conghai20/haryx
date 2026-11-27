// keycloak-js wrapper. Tokens live in memory only (keycloak-js keeps them on its instance; nothing
// is written to storage). Flow: PKCE S256, check-sso by redirect (no iframe), so a reload keeps an
// SSO session signed in and a sign-out lands on the signed-out screen instead of bouncing to login.
import Keycloak from "keycloak-js";
import { config } from "./config";
import { createApi } from "./api";
import { startRefresh } from "./refresh";

export type AuthStatus = "starting" | "signedOut" | "signedIn" | "failed";

class Auth {
  status = $state<AuthStatus>("starting");
  email = $state<string | null>(null);
  private kc = new Keycloak({ url: config.url, realm: config.realm, clientId: config.clientId });
  private stop: (() => void) | null = null;

  async init() {
    try {
      const ok = await this.kc.init({ onLoad: "check-sso", pkceMethod: "S256", checkLoginIframe: false });
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
