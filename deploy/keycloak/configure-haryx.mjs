// Registers HaryX in the shared WeCoAp realm, through Keycloak's admin REST API. Idempotent: safe
// to rerun after any change here. Written in the style of wecoap-sso/scripts/configure-realm.mjs,
// which must have run first (consent attributes, locales, events).
//
//   set -a; . deploy/.env; set +a
//   KC_ADMIN_USER=… KC_ADMIN_PASSWORD=… node deploy/keycloak/configure-haryx.mjs
//
// The realm is shared with AR Onion (plan: "Keycloak realm wecoap"), so this script:
//  - creates or updates only clients whose id starts with `haryx-`;
//  - never touches the `github` and `google` identity providers, which the realm owner set up for
//    every product; it only reports whether they are there;
//  - sets realm-wide only what HaryX needs and every product benefits from: verify email, brute
//    force detection, and SMTP through mail.wecoap.com.
//
// Clients:
//  - haryx-cli        public, PKCE S256, loopback redirect, device grant, offline sessions
//  - haryx-platform   public, PKCE S256, the /platform/ SPA, normal sessions only
//  - haryx-api-admin  confidential service account for the API: view-users + manage-users, and the
//                     kc-admin-api scope Keycloak 26.8.2 requires for Admin API calls (plan A3)
import { readFileSync } from "node:fs";

const env = (name, fallback) => {
    const value = process.env[name] || fallback;
    if (!value) throw new Error(`${name} is not set (see the header of this script)`);
    return value;
};
const optional = name => process.env[name] || "";

// The master admin password goes to KC_URL, and PLATFORM_URL becomes a redirect URI: https only,
// except for a Keycloak or dev server on this machine.
const secureUrl = (name, fallback) => {
    const url = new URL(env(name, fallback));
    const local = ["127.0.0.1", "localhost"].includes(url.hostname);
    if (url.protocol !== "https:" && !local) throw new Error(`${name} must be https:// (got ${url.href})`);
    if (url.username || url.password || url.search || url.hash) throw new Error(`${name} must be a plain origin (got ${url.href})`);
    return url.href.replace(/\/$/, "");
};
const KC_URL = secureUrl("KC_URL", "https://sso.wecoap.com");
const REALM = env("KC_REALM", "wecoap");
const PLATFORM_URL = secureUrl("HARYX_PLATFORM_URL", "https://haryx.wecoap.com");
const API_ADMIN_SECRET = env("HARYX_API_ADMIN_SECRET");
const API_AUDIENCE = "haryx-api";
const ADMIN_ROLES = ["view-users", "manage-users"];
const SMTP = { host: "mail.wecoap.com", port: "587", from: "no-reply@wecoap.com", fromDisplayName: "WeCoAp" };
const SHARED_PROVIDERS = ["github", "google"];

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

const smtpUser = optional("SMTP_USER");
const smtpPassword = optional("SMTP_PASSWORD");
const otherSmtp = realm.smtpServer?.host && realm.smtpServer.host !== SMTP.host;
if (otherSmtp) {
    done.push(`SMTP left as is: the realm sends through ${realm.smtpServer.host}, not ${SMTP.host}`);
} else if (smtpUser && smtpPassword) {
    realmPatch.smtpServer = {
        ...SMTP,
        replyTo: SMTP.from,
        envelopeFrom: SMTP.from,
        starttls: "true",
        ssl: "false",
        auth: "true",
        user: smtpUser,
        password: smtpPassword
    };
} else {
    done.push(`SMTP skipped: SMTP_USER / SMTP_PASSWORD not set (realm keeps ${realm.smtpServer?.host || "no SMTP"})`);
}
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
    },
    {
        rep: {
            clientId: "haryx-api-admin",
            name: "HaryX API (service account)",
            protocol: "openid-connect",
            publicClient: false,
            clientAuthenticatorType: "client-secret",
            secret: API_ADMIN_SECRET,
            serviceAccountsEnabled: true,
            standardFlowEnabled: false,
            implicitFlowEnabled: false,
            directAccessGrantsEnabled: false,
            consentRequired: false,
            fullScopeAllowed: false,
            ...resetFields,
            redirectUris: [],
            webOrigins: [],
            attributes: { ...resetAttributes, "oauth2.device.authorization.grant.enabled": "false" }
        },
        mappers: [],
        defaultScopes: ["kc-admin-api"],
        optionalScopes: { add: [], remove: [] }
    }
];

async function upsertClient({ rep, mappers, defaultScopes = [], optionalScopes }) {
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

    for (const name of defaultScopes) await api("PUT", `/clients/${client.id}/default-client-scopes/${scopeId(name)}`);
    for (const name of optionalScopes.add) await api("PUT", `/clients/${client.id}/optional-client-scopes/${scopeId(name)}`);
    const optional = await api("GET", `/clients/${client.id}/optional-client-scopes`);
    for (const name of optionalScopes.remove) {
        if (optional.some(s => s.name === name)) await api("DELETE", `/clients/${client.id}/optional-client-scopes/${scopeId(name)}`);
    }
    return client;
}

const clients = {};
for (const spec of CLIENTS) clients[spec.rep.clientId] = await upsertClient(spec);

// ---------- the API's service account: exactly view-users + manage-users ----------
// fullScopeAllowed is off, so a role reaches the token only if it is both granted to the
// service-account user and in the client's scope.
const apiAdmin = clients["haryx-api-admin"];
const serviceUser = await api("GET", `/clients/${apiAdmin.id}/service-account-user`);
const [realmManagement] = await api("GET", "/clients?clientId=realm-management");
const roles = (await api("GET", `/clients/${realmManagement.id}/roles`)).filter(r => ADMIN_ROLES.includes(r.name));
if (roles.length !== ADMIN_ROLES.length) throw new Error(`realm-management is missing one of ${ADMIN_ROLES}`);
await api("POST", `/users/${serviceUser.id}/role-mappings/clients/${realmManagement.id}`, roles);
await api("POST", `/clients/${apiAdmin.id}/scope-mappings/clients/${realmManagement.id}`, roles);
const extra = (await api("GET", `/users/${serviceUser.id}/role-mappings/clients/${realmManagement.id}`))
    .filter(r => !ADMIN_ROLES.includes(r.name));
if (extra.length) await api("DELETE", `/users/${serviceUser.id}/role-mappings/clients/${realmManagement.id}`, extra);

// ---------- identity providers ----------
const providers = await api("GET", "/identity-provider/instances");
for (const alias of SHARED_PROVIDERS) {
    const found = providers.find(p => p.alias === alias);
    done.push(`${alias}: ${found ? (found.enabled ? "present, left as is" : "present but DISABLED, left as is") : "MISSING (the realm owner adds it)"}`);
}

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

console.log(`realm ${REALM} at ${KC_URL}: clients ${Object.keys(clients).join(", ")}; verify email and brute force on`);
for (const line of done) console.log(`  ${line}`);
