// Sets up HaryX's clients in a LOCAL Keycloak 26.8.2, for tests (task 13's end-to-end run, the
// API's integration tests). Production is configured by hand from the checklist in
// deploy/README.md ("Realm, by hand"); this script mirrors that checklist and must change with it.
// It refuses any Keycloak that is not on this machine. Idempotent.
//
//   KC_URL=http://127.0.0.1:18083 KC_ADMIN_USER=admin KC_ADMIN_PASSWORD=… \
//     node deploy/keycloak/configure-local.mjs        (test-local.sh does this for you)
//
// Clients:
//  - haryx-cli        public, PKCE S256, loopback redirect, device grant, offline sessions
//  - haryx-platform   public, PKCE S256, the /platform/ SPA, normal sessions only, and the
//                     account roles that let a user end their own machines' sessions (plan A7)
// No admin service account: HaryX holds no Keycloak secret.
import { readFileSync } from "node:fs";

const env = (name, fallback) => {
    const value = process.env[name] || fallback;
    if (!value) throw new Error(`${name} is not set (see the header of this script)`);
    return value;
};
const optional = name => process.env[name] || "";

// Only a plain http://127.0.0.1:<port>: this script must never reach sso.wecoap.com.
const localUrl = (name, fallback) => {
    const url = new URL(env(name, fallback));
    if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || !url.port || url.username || url.password || url.pathname !== "/") {
        throw new Error(`${name} must be a local http://127.0.0.1:<port> (got ${url.href}); production is set up by hand`);
    }
    return url.origin;
};
const KC_URL = localUrl("KC_URL");
const REALM = env("KC_REALM", "wecoap");
const PLATFORM_URL = localUrl("HARYX_PLATFORM_URL", "http://127.0.0.1:5173");
const API_AUDIENCE = "haryx-api";
// What haryx-platform's token may do in the account API: the user's own account only.
const ACCOUNT_ROLES = ["manage-account", "view-profile"];

async function adminToken() {
    const res = await fetch(`${KC_URL}/realms/master/protocol/openid-connect/token`, {
        method: "POST",
        body: new URLSearchParams({
            client_id: "admin-cli",
            grant_type: "password",
            username: env("KC_ADMIN_USER"),
            password: env("KC_ADMIN_PASSWORD")
        })
    });
    if (!res.ok) throw new Error(`admin sign-in failed: ${res.status} ${await res.text()}`);
    return (await res.json()).access_token;
}

const token = await adminToken();
async function api(method, path, body, base = `${KC_URL}/admin/realms/${REALM}`) {
    const res = await fetch(`${base}${path}`, {
        method,
        headers: { Authorization: `Bearer ${token}`, ...(body ? { "Content-Type": "application/json" } : {}) },
        body: body ? JSON.stringify(body) : undefined
    });
    if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
    const text = await res.text();
    return text ? JSON.parse(text) : null;
}

const done = [];

// ---------- realm-wide settings HaryX relies on ----------
const realm = await api("GET", "");
const realmPatch = {};
if (!realm.verifyEmail) realmPatch.verifyEmail = true;
if (!realm.bruteForceProtected) realmPatch.bruteForceProtected = true;

if (Object.keys(realmPatch).length) await api("PUT", "", realmPatch);

// ---------- client scopes ----------
const scopes = await api("GET", "/client-scopes");
const scopeId = name => {
    const scope = scopes.find(s => s.name === name);
    if (!scope) throw new Error(`client scope ${name} is missing from realm ${REALM}`);
    return scope.id;
};

// ---------- clients ----------
const audienceMapper = {
    name: "audience haryx-api",
    protocol: "openid-connect",
    protocolMapper: "oidc-audience-mapper",
    config: { "included.custom.audience": API_AUDIENCE, "access.token.claim": "true", "id.token.claim": "false", "introspection.token.claim": "true" }
};
// Which way the user signed in (github, google, apple); absent for email + password (plan A5).
const providerMapper = {
    name: "identity_provider",
    protocol: "openid-connect",
    protocolMapper: "oidc-usersessionmodel-note-mapper",
    config: {
        "user.session.note": "identity_provider",
        "claim.name": "identity_provider",
        "jsonType.label": "String",
        "access.token.claim": "true",
        "id.token.claim": "true",
        "introspection.token.claim": "true"
    }
};

// Fields a hand edit could add and a merge would keep: every run states them.
const resetFields = { rootUrl: "", baseUrl: "", adminUrl: "" };
const resetAttributes = { "post.logout.redirect.uris": "", "frontchannel.logout.url": "", "backchannel.logout.url": "" };

const publicClient = {
    ...resetFields,
    publicClient: true,
    standardFlowEnabled: true,
    implicitFlowEnabled: false,
    directAccessGrantsEnabled: false,
    serviceAccountsEnabled: false,
    consentRequired: false,
    fullScopeAllowed: false,
    protocol: "openid-connect"
};

const CLIENTS = [
    {
        rep: {
            ...publicClient,
            clientId: "haryx-cli",
            name: "HaryX CLI and apps",
            // Any loopback port and path; Phase 0 item 1 checked what this refuses.
            redirectUris: ["http://127.0.0.1/*"],
            webOrigins: [],
            attributes: {
                ...resetAttributes,
                "pkce.code.challenge.method": "S256",
                // `--no-browser` over SSH. PKCE applies to it too (plan A4).
                "oauth2.device.authorization.grant.enabled": "true",
                "frontchannel.logout.session.required": "false"
            }
        },
        mappers: [audienceMapper, providerMapper],
        optionalScopes: { add: ["offline_access"], remove: [] }
    },
    {
        rep: {
            ...publicClient,
            clientId: "haryx-platform",
            name: "HaryX on the web",
            rootUrl: PLATFORM_URL,
            baseUrl: "/platform/",
            redirectUris: [`${PLATFORM_URL}/platform/*`],
            webOrigins: [PLATFORM_URL],
            attributes: {
                ...resetAttributes,
                "pkce.code.challenge.method": "S256",
                "oauth2.device.authorization.grant.enabled": "false",
                "post.logout.redirect.uris": `${PLATFORM_URL}/platform/*`
            }
        },
        mappers: [audienceMapper, providerMapper],
        // The browser holds no long-lived session: no offline tokens.
        optionalScopes: { add: [], remove: ["offline_access"] }
    }
];

async function upsertClient({ rep, mappers, optionalScopes }) {
    if (!rep.clientId.startsWith("haryx-")) throw new Error(`refusing to touch ${rep.clientId}: not a haryx- client`);
    let [client] = await api("GET", `/clients?clientId=${encodeURIComponent(rep.clientId)}`);
    if (!client) {
        await api("POST", "/clients", rep);
        [client] = await api("GET", `/clients?clientId=${encodeURIComponent(rep.clientId)}`);
    }
    // Always PUT, new client included: on create Keycloak fills empty redirect lists with "/*".
    await api("PUT", `/clients/${client.id}`, { ...client, ...rep, attributes: { ...client.attributes, ...rep.attributes } });

    const existing = await api("GET", `/clients/${client.id}/protocol-mappers/models`);
    for (const mapper of mappers) {
        const found = existing.find(m => m.name === mapper.name);
        if (found) await api("PUT", `/clients/${client.id}/protocol-mappers/models/${found.id}`, { ...found, ...mapper });
        else await api("POST", `/clients/${client.id}/protocol-mappers/models`, mapper);
    }

    for (const name of optionalScopes.add) await api("PUT", `/clients/${client.id}/optional-client-scopes/${scopeId(name)}`);
    const optional = await api("GET", `/clients/${client.id}/optional-client-scopes`);
    for (const name of optionalScopes.remove) {
        if (optional.some(s => s.name === name)) await api("DELETE", `/clients/${client.id}/optional-client-scopes/${scopeId(name)}`);
    }
    return client;
}

const clients = {};
for (const spec of CLIENTS) clients[spec.rep.clientId] = await upsertClient(spec);

// ---------- haryx-platform: the user's own account, through the account API (plan A7) ----------
// fullScopeAllowed is off, so the account roles reach its token only through this scope mapping.
const platform = clients["haryx-platform"];
const [account] = await api("GET", "/clients?clientId=account");
const accountRoles = (await api("GET", `/clients/${account.id}/roles`)).filter(r => ACCOUNT_ROLES.includes(r.name));
if (accountRoles.length !== ACCOUNT_ROLES.length) throw new Error(`the account client is missing one of ${ACCOUNT_ROLES}`);
await api("POST", `/clients/${platform.id}/scope-mappings/clients/${account.id}`, accountRoles);
const extraScope = (await api("GET", `/clients/${platform.id}/scope-mappings/clients/${account.id}`)).filter(r => !ACCOUNT_ROLES.includes(r.name));
if (extraScope.length) await api("DELETE", `/clients/${platform.id}/scope-mappings/clients/${account.id}`, extraScope);

// ---------- identity providers ----------
const providers = await api("GET", "/identity-provider/instances");

const apple = {
    servicesId: optional("APPLE_SERVICES_ID"),
    teamId: optional("APPLE_TEAM_ID"),
    keyId: optional("APPLE_KEY_ID"),
    p8File: optional("APPLE_P8_FILE")
};
if (Object.values(apple).every(Boolean)) {
    const serverInfo = await api("GET", "/serverinfo", null, `${KC_URL}/admin`);
    const loaded = Object.keys(serverInfo.providers?.social?.providers ?? {}).includes("apple");
    if (!loaded) throw new Error("the apple provider is not loaded: mount apple-identity-provider-1.17.0.jar in wecoap-sso/deploy (plan A6)");
    // The extension takes the key body: no PEM armour, no line breaks.
    const p8 = readFileSync(apple.p8File, "utf8").replace(/-----[^-]+-----/g, "").replace(/\s+/g, "");
    const rep = {
        alias: "apple",
        providerId: "apple",
        displayName: "Apple",
        enabled: true,
        // Off on purpose: with it on, Keycloak's first-broker-login links an Apple sign-in to any
        // existing account with that email without asking that account to prove it owns it.
        trustEmail: false,
        storeToken: false,
        config: { clientId: apple.servicesId, teamId: apple.teamId, keyId: apple.keyId, clientSecret: p8, defaultScope: "name email" }
    };
    const found = providers.find(p => p.alias === "apple");
    if (found) await api("PUT", "/identity-provider/instances/apple", { ...found, ...rep, config: { ...found.config, ...rep.config } });
    else await api("POST", "/identity-provider/instances", rep);
    done.push(`apple: ${found ? "updated" : "created"} (Services ID ${apple.servicesId})`);
} else {
    const present = providers.some(p => p.alias === "apple");
    done.push(`apple skipped: APPLE_SERVICES_ID / APPLE_TEAM_ID / APPLE_KEY_ID / APPLE_P8_FILE not all set${present ? " (existing provider left as is)" : ""}`);
}

console.log(`local realm ${REALM} at ${KC_URL}: clients ${Object.keys(clients).join(", ")}; verify email and brute force on`);
for (const line of done) console.log(`  ${line}`);
