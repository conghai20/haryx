// Checks configure-local.mjs against a throwaway Keycloak. Driven by test-local.sh; never point it
// at sso.wecoap.com (`setup` deletes and recreates the realm).
//
//   node test-haryx.mjs setup            fresh realm + a verified test user
//   node test-haryx.mjs snapshot FILE    what configure-local.mjs owns, as stable JSON
//   node test-haryx.mjs tamper           hand edits a rerun must undo
//   node test-haryx.mjs check            sign-ins, tokens, device grant, remote sign-out
import crypto from "node:crypto";
import { writeFileSync } from "node:fs";

const KC = process.env.KC_URL;
const REALM = process.env.KC_REALM;
const OIDC = `${KC}/realms/${REALM}/protocol/openid-connect`;
const USER = { email: "tester@example.com", password: "tester-password-1" };
// `setup` deletes the realm: refuse anything but a plain http://127.0.0.1:<port>.
const target = new URL(KC ?? "http://invalid");
if (target.protocol !== "http:" || target.hostname !== "127.0.0.1" || !target.port || target.username || target.password || target.pathname !== "/") {
    throw new Error(`KC_URL must be a throwaway local Keycloak like http://127.0.0.1:18083, got ${KC}`);
}

async function adminToken() {
    const res = await fetch(`${KC}/realms/master/protocol/openid-connect/token`, {
        method: "POST",
        body: new URLSearchParams({ grant_type: "password", client_id: "admin-cli", username: process.env.KC_ADMIN_USER, password: process.env.KC_ADMIN_PASSWORD })
    });
    return (await res.json()).access_token;
}
async function api(method, path, body, token) {
    const res = await fetch(`${KC}/admin/realms${path}`, {
        method,
        headers: { Authorization: `Bearer ${token ?? (await adminToken())}`, ...(body ? { "Content-Type": "application/json" } : {}) },
        body: body ? JSON.stringify(body) : undefined
    });
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null };
}

const failures = [];
const check = (name, ok, detail = "") => {
    console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
    if (!ok) failures.push(name);
};

// ---------- setup ----------
async function setup() {
    await api("DELETE", `/${REALM}`);
    await api("POST", "", { realm: REALM, enabled: true, registrationEmailAsUsername: true, loginWithEmailAllowed: true });
    await api("POST", `/${REALM}/users`, {
        username: USER.email, email: USER.email, emailVerified: true, enabled: true, firstName: "Test", lastName: "User",
        credentials: [{ type: "password", value: USER.password, temporary: false }]
    });
    console.log(`realm ${REALM} recreated with ${USER.email}`);
}

// ---------- snapshot ----------
const sortBy = key => (a, b) => String(a[key]).localeCompare(String(b[key]));
const strip = ({ id, ...rest }) => rest;
async function snapshot(file) {
    const token = await adminToken();
    const get = async path => (await api("GET", `/${REALM}${path}`, null, token)).json;
    const out = {};
    const realm = await get("");
    out.realm = { verifyEmail: realm.verifyEmail, bruteForceProtected: realm.bruteForceProtected };
    const [account] = await get("/clients?clientId=account");
    for (const clientId of ["haryx-cli", "haryx-platform"]) {
        const [c] = await get(`/clients?clientId=${clientId}`);
        const { id, ...rep } = c;
        out[clientId] = {
            rep,
            accountScope: (await get(`/clients/${id}/scope-mappings/clients/${account.id}`)).map(r => r.name).sort(),
            mappers: (await get(`/clients/${id}/protocol-mappers/models`)).map(strip).sort(sortBy("name")),
            defaultScopes: (await get(`/clients/${id}/default-client-scopes`)).map(s => s.name).sort(),
            optionalScopes: (await get(`/clients/${id}/optional-client-scopes`)).map(s => s.name).sort()
        };
    }
    out.adminClients = (await get("/clients")).map(c => c.clientId).filter(id => id.startsWith("haryx-")).sort();
    out.providers = (await get("/identity-provider/instances")).map(p => ({ alias: p.alias, enabled: p.enabled, trustEmail: p.trustEmail, config: { ...p.config, clientSecret: p.config.clientSecret ? "set" : "" } })).sort(sortBy("alias"));
    writeFileSync(file, JSON.stringify(out, null, 2));
    console.log(`snapshot → ${file}`);
}

// ---------- tamper: what a hand edit in the admin console might leave behind ----------
async function tamper() {
    const token = await adminToken();
    const [c] = (await api("GET", `/${REALM}/clients?clientId=haryx-cli`, null, token)).json;
    await api("PUT", `/${REALM}/clients/${c.id}`, {
        ...c,
        redirectUris: [...c.redirectUris, "https://evil.example/*"],
        directAccessGrantsEnabled: true,
        attributes: { ...c.attributes, "post.logout.redirect.uris": "https://evil.example/*", "backchannel.logout.url": "https://evil.example/bc" }
    }, token);
    console.log("tampered with haryx-cli");
}

// ---------- browser-ish sign-in ----------
class Jar {
    cookies = new Map();
    take(res) { for (const c of res.headers.getSetCookie()) { const [kv] = c.split(";"); const i = kv.indexOf("="); this.cookies.set(kv.slice(0, i), kv.slice(i + 1)); } }
    header() { return [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; "); }
}
const b64url = b => Buffer.from(b).toString("base64url");
const pkce = () => { const verifier = b64url(crypto.randomBytes(32)); return { verifier, challenge: b64url(crypto.createHash("sha256").update(verifier).digest()) }; };
const claims = jwt => JSON.parse(Buffer.from(jwt.split(".")[1], "base64url"));
const token = async params => { const r = await fetch(`${OIDC}/token`, { method: "POST", body: new URLSearchParams(params) }); return { status: r.status, json: await r.json() }; };

async function follow(res, jar, steps = 5) {
    for (let i = 0; i < steps && res.status >= 300 && res.status < 400; i++) {
        const loc = res.headers.get("location");
        if (!loc.startsWith(KC)) return res;
        res = await fetch(loc, { redirect: "manual", headers: { cookie: jar.header() } });
        jar.take(res);
    }
    return res;
}
async function submitLogin(res, jar) {
    const html = await res.text();
    const action = html.match(/action="([^"]+)"/)?.[1]?.replaceAll("&amp;", "&");
    if (!action) throw new Error(`no login form (HTTP ${res.status})`);
    const out = await fetch(action, {
        method: "POST", redirect: "manual",
        headers: { cookie: jar.header(), "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({ username: USER.email, password: USER.password, credentialId: "" })
    });
    jar.take(out);
    return out;
}

async function codeFlow(clientId, redirect, scope) {
    const jar = new Jar();
    const { verifier, challenge } = pkce();
    const url = `${OIDC}/auth?${new URLSearchParams({ client_id: clientId, response_type: "code", scope, redirect_uri: redirect, state: "s", code_challenge: challenge, code_challenge_method: "S256" })}`;
    let res = await fetch(url, { redirect: "manual" });
    jar.take(res);
    res = await submitLogin(res, jar);
    const loc = res.headers.get("location") ?? "";
    if (!loc.startsWith(redirect)) throw new Error(`${clientId}: sign-in ended at ${res.status} ${loc}`);
    const code = new URL(loc).searchParams.get("code");
    return token({ grant_type: "authorization_code", client_id: clientId, code, redirect_uri: redirect, code_verifier: verifier });
}

// ---------- check ----------
async function run() {
    // haryx-cli: loopback, offline, audience
    const port = 40000 + crypto.randomInt(20000);
    const cli = await codeFlow("haryx-cli", `http://127.0.0.1:${port}/callback/${b64url(crypto.randomBytes(8))}`, "openid offline_access");
    check("haryx-cli signs in to a random loopback port", cli.status === 200, `HTTP ${cli.status}`);
    const at = claims(cli.json.access_token);
    check("haryx-cli access token: aud has haryx-api, azp haryx-cli, sid set", [].concat(at.aud).includes("haryx-api") && at.azp === "haryx-cli" && !!at.sid, `aud=${JSON.stringify(at.aud)}`);
    check("haryx-cli refresh token is offline", claims(cli.json.refresh_token).typ === "Offline");
    check("password sign-in has no identity_provider claim (whoami: email)", at.identity_provider === undefined);
    const refused = await fetch(`${OIDC}/auth?${new URLSearchParams({ client_id: "haryx-cli", response_type: "code", scope: "openid", redirect_uri: "http://localhost:5/callback/x", code_challenge: pkce().challenge, code_challenge_method: "S256" })}`, { redirect: "manual" });
    check("haryx-cli refuses a localhost redirect", refused.status === 400, `HTTP ${refused.status}`);

    // haryx-platform: no offline tokens, audience
    const webRedirect = `${process.env.HARYX_PLATFORM_URL}/platform/callback`;
    const askOffline = await fetch(`${OIDC}/auth?${new URLSearchParams({ client_id: "haryx-platform", response_type: "code", scope: "openid offline_access", redirect_uri: webRedirect, code_challenge: pkce().challenge, code_challenge_method: "S256" })}`, { redirect: "manual" });
    const askOfflineAt = askOffline.headers.get("location") ?? "";
    check("haryx-platform asking for offline_access is refused", askOfflineAt.startsWith(webRedirect) && /error=invalid_scope/.test(askOfflineAt), askOfflineAt.replace(webRedirect, "…").slice(0, 80));
    const web = await codeFlow("haryx-platform", webRedirect, "openid");
    const wat = web.json.access_token ? claims(web.json.access_token) : {};
    const wrt = web.json.refresh_token ? claims(web.json.refresh_token) : {};
    check("haryx-platform signs in; aud has haryx-api", web.status === 200 && [].concat(wat.aud).includes("haryx-api"), `HTTP ${web.status}`);
    check("haryx-platform's refresh token is not offline", wrt.typ !== "Offline", `refresh typ ${wrt.typ}`);
    const otherSite = await fetch(`${OIDC}/auth?${new URLSearchParams({ client_id: "haryx-platform", response_type: "code", scope: "openid", redirect_uri: "https://evil.example/platform/x", code_challenge: pkce().challenge, code_challenge_method: "S256" })}`, { redirect: "manual" });
    check("haryx-platform refuses another site's redirect", otherSite.status === 400, `HTTP ${otherSite.status}`);

    // device grant with PKCE (plan A4)
    const dp = pkce();
    const start = await (await fetch(`${OIDC}/auth/device`, { method: "POST", body: new URLSearchParams({ client_id: "haryx-cli", scope: "openid offline_access", code_challenge: dp.challenge, code_challenge_method: "S256" }) })).json();
    check("device grant starts with PKCE", !!start.device_code, start.error_description ?? "");
    const jar = new Jar();
    let res = await fetch(start.verification_uri_complete, { redirect: "manual" });
    jar.take(res);
    res = await follow(await submitLogin(await follow(res, jar), jar), jar);
    // Keycloak asks "grant access to this device?" even with consentRequired off.
    const grant = await res.text();
    const grantAction = grant.match(/action="([^"]+)"/)?.[1]?.replaceAll("&amp;", "&");
    check("device sign-in shows the grant screen", !!grantAction && /accept/i.test(grant));
    if (grantAction) {
        const accepted = await fetch(new URL(grantAction, KC).href, { method: "POST", redirect: "manual", headers: { cookie: jar.header(), "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ accept: "Yes" }) });
        jar.take(accepted);
    }
    await new Promise(r => setTimeout(r, (start.interval + 0.5) * 1000));
    const device = await token({ grant_type: "urn:ietf:params:oauth:grant-type:device_code", client_id: "haryx-cli", device_code: start.device_code, code_verifier: dp.verifier });
    check("device grant returns offline tokens", device.status === 200 && claims(device.json.refresh_token).typ === "Offline", device.json.error ?? "");

    // Sign out one machine from the web, with the user's own token (plan A7)
    check("no haryx-api-admin client exists", !(await api("GET", `/${REALM}/clients?clientId=haryx-api-admin`)).json.length);
    check("haryx-platform token can use the account API (aud account, manage-account)",
        [].concat(wat.aud).includes("account") && (wat.resource_access?.account?.roles ?? []).includes("manage-account"),
        `aud=${JSON.stringify(wat.aud)} roles=${JSON.stringify(wat.resource_access?.account?.roles)}`);
    const accountApi = (method, path, bearer = web.json.access_token) =>
        fetch(`${KC}/realms/${REALM}/account${path}`, { method, headers: { authorization: `Bearer ${bearer}`, accept: "application/json" } });
    const devices = await accountApi("GET", "/sessions/devices");
    const sessions = devices.ok ? (await devices.json()).flatMap(d => d.sessions ?? []) : [];
    check("the user sees their CLI machine among their sessions", sessions.some(x => x.id === at.sid), `${sessions.length} sessions`);
    const signOut = await accountApi("DELETE", `/sessions/${at.sid}`);
    check("the web signs out that one machine", signOut.status === 204, `HTTP ${signOut.status}`);
    const gone = await token({ grant_type: "refresh_token", client_id: "haryx-cli", refresh_token: cli.json.refresh_token });
    const kept = await token({ grant_type: "refresh_token", client_id: "haryx-cli", refresh_token: device.json.refresh_token });
    check("that machine is signed out (invalid_grant)", gone.json.error === "invalid_grant", gone.json.error);
    check("the other machine still refreshes", kept.status === 200, `HTTP ${kept.status}`);
    const cliAccount = await accountApi("GET", "/sessions/devices", kept.json.access_token);
    check("a haryx-cli token cannot use the account API", cliAccount.status === 401 || cliAccount.status === 403, `HTTP ${cliAccount.status}`);
    const adminCall = await api("GET", `/${REALM}/users`, null, web.json.access_token);
    check("a haryx-platform token cannot use the Admin API", adminCall.status === 401 || adminCall.status === 403, `HTTP ${adminCall.status}`);

    console.log(failures.length ? `\n${failures.length} FAILED` : "\nall passed");
    process.exit(failures.length ? 1 : 0);
}

const [cmd, arg] = process.argv.slice(2);
if (cmd === "setup") await setup();
else if (cmd === "snapshot") await snapshot(arg);
else if (cmd === "tamper") await tamper();
else if (cmd === "check") await run();
else throw new Error("usage: test-haryx.mjs setup | snapshot FILE | tamper | check");
