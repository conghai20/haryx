# HaryX on WeCoAp SSO, a HaryX API in Vietnam, and the web platform: plan

Status: **approved** (2026-11-26), not started. Prerequisites P1–P4 are on the owner. Supersedes the backend half of
`haryx-account-plan.md` (Supabase Auth, PostgREST, Storage, Edge Function). The sync engine,
encryption and merge rules from that plan stay as built.

## Decisions

Taken with the owner on 2026-11-26:

| Topic | Decision |
|---|---|
| Identity | **Keycloak (WeCoAp SSO) is the only token issuer.** Supabase Auth is removed. |
| Realm | **`https://sso.wecoap.com`, realm `wecoap`**, shared by the WeCoAp consumer products (HaryX, AR Onion, and later ones): one WeCoAp ID, single sign-on across them, one client per product. **WeCoMed is not in it.** It keeps its own identity system (plan 054: phone, TOTP, tenants, enterprise SSO) because it holds health data and serves B2B tenants. It may later offer "Sign in with WeCoAp" as an identity provider, without merging the user stores. |
| Backend | **A HaryX API in Go** (`server/api/`) checks Keycloak JWTs and owns its own Postgres. Supabase is removed entirely. |
| Hosting | **Everything in Vietnam**, on the aaPanel server at `160.187.0.66` (also `mail.wecoap.com`): Keycloak and its Postgres (live), and the HaryX API, its own Postgres and the web app. No personal data is stored abroad. |
| Sign-in methods | **GitHub, Google, Apple, email + password** (with email verification), all through Keycloak. |
| Web platform | `https://haryx.wecoap.com/platform/`: a **Vite + Svelte SPA** with its own deploy. Sections: Machines, Accounts, Usage, Profiles, Privacy center. |
| Machines | A **plaintext machine registry** (name, OS, HaryX version, last seen). The web can **rename** a machine and **sign it out**. It sends no other remote commands. |
| Encrypted data on the web | **Unlocked in the browser** with the sync passphrase. The server stays blind. |
| Usage | **Syncs end-to-end encrypted** and is shown on the web only after unlocking. |
| Consent (Law 91/2025/QH15) | The realm-wide Terms and Privacy consent already in `wecoap-sso`, plus **two separate, withdrawable HaryX consents**: *Sync* (usage included) and *Machine registry*. Each is asked the first time its feature is turned on. |
| Edge | **No Cloudflare proxy** in front of personal data:
- `sso.wecoap.com` and `haryx.wecoap.com` are **DNS-only**. TLS ends on the VN server, with
  Let's Encrypt certificates. `sso.wecoap.com` is already switched (checked 2026-11-26);
  `haryx.wecoap.com` is still proxied until task 1.
- The marketing site keeps reaching GitHub Pages through that server's nginx, as today. |
| Mail | The self-hosted **`mail.wecoap.com`**, with an existing `no-reply@wecoap.com` account. |
| Deploy | **Keycloak is not HaryX's.** It is deployed from `wecoap-sso/deploy/` (aaPanel Docker Compose, Keycloak 26.8.2, its own Postgres 18, aaPanel nginx and Let's Encrypt), live since 2026-11-26. HaryX's `deploy/` holds only the HaryX API, its Postgres and the `haryx.wecoap.com` site, deployed the same way. |
| Migration | **None.** Sign-in was never released. The Supabase project `haryx-dev` and its test data are dropped. |

Defaults this plan takes (change any before approval):

- **Account linking** uses Keycloak's standard first-broker-login: when a GitHub, Google or Apple
  identity's email matches an existing user, the user confirms by signing in to that user (or by
  email). It is not linked silently. Apple's private-relay emails never match, so those users link
  from the Keycloak account console.
- **Usage that syncs** is per account per day (tokens by model, plus 5h/7d quota readings). Project
  paths, session ids and transcript content never sync.
- **Profile files** live in the HaryX Postgres as `bytea` (3 × 10 MB per user, as today), so one
  backup covers everything. They move to object storage if the volume grows.
- **Mail** goes through the existing WeCoAp mail server, **`mail.wecoap.com`**:
  - Authenticated submission on 587 with STARTTLS, as the existing `no-reply@wecoap.com` account.
  - Never sent straight from the Keycloak host. `wecoap.com` publishes SPF `+mx +a` and DMARC
    `p=quarantine`, so direct mail would be quarantined.
- **Apple sign-in** uses the existing Apple Developer account (the macOS app's team).
- **Offsite backups** stay in Vietnam too.

## The user-facing flow

1. **Sign-up.**
   - The user clicks *Continue with GitHub / Google / Apple*, or *Sign in with email*, in the macOS
     or Windows app, or runs `haryx login`.
   - The browser opens on the WeCoAp sign-in page (vi by default, en available). New users tick
     Terms and Privacy, each one a separate box with the document version recorded.
   - Keycloak returns them to HaryX.
2. **Turning on sync.** The app asks for the *Sync* consent: what is uploaded, that it is
   end-to-end encrypted, that it is stored in Vietnam, and a link to the policy version. Only then
   does it ask for the sync passphrase, as today.
3. **Showing this machine on the web.** A separate toggle, "Show this Mac on HaryX web", asks for
   the *Machine registry* consent. While it is off, the machine syncs but does not appear in the
   Machines list. Its Keycloak session still shows in the WeCoAp account console, because that is
   part of signing in.
4. **On the web.**
   - The user signs in at `/platform/` and sees Machines and the Privacy center straight away.
   - Accounts, Usage and Profiles say "Locked: enter your sync passphrase". Unlocking decrypts in
     the tab, keeps the key in memory only, and locks again after 15 minutes idle or when the tab
     closes.
5. **Signing out a machine from the web.**
   - The API revokes that machine's Keycloak session and refuses its tokens straight away.
   - The machine's next sync or renewal says "Signed out of HaryX from the web", with *Sign in*.
     Everything local keeps working.
6. **Withdrawing a consent** (Privacy center, the app, or `haryx account consent withdraw`).
   - Withdrawing Sync stops sync on every machine and deletes the server copy (rows, profiles and
     usage).
   - Withdrawing Machine registry deletes the machine rows.
   - Both are logged with a date.
7. **Export / delete account** (Privacy center or the CLI).
   - Export downloads one JSON file: the Keycloak profile, consents and their history, machines,
     and the ciphertext rows.
   - Delete account opens the WeCoAp account page (`https://sso.wecoap.com/realms/wecoap/account/`),
     where WeCoAp SSO deletes the WeCoAp ID. HaryX has no delete endpoint of its own (A10). To
     erase HaryX data and keep the WeCoAp ID, the user withdraws both consents.

When something fails:
- **SSO down:** "Can't reach WeCoAp sign-in". Local features keep working.
- **API down:** "Sync unavailable, will retry".
- **Consent missing:** the API answers 403 `consent_required`, and the app asks for it again.
- **Policy version bumped:** Keycloak's `VERIFY_PROFILE` stops the user at the consent screen on
  their next interactive sign-in. The CLI refresh keeps working until then, and the apps show
  "Review the updated terms" with a link.

## Architecture

```
                 ┌──────────── aaPanel server 160.187.0.66 (VN) ───────────────────────┐
CLI / macOS /    │ nginx ── sso.wecoap.com ──────────▶ Keycloak 26.8.2 ─▶ Postgres 18     │
Windows app ─────┼──▶     │                            (wecoap-sso/deploy, live)           │
Browser ─────────┼──▶     ── haryx.wecoap.com/api/ ───▶ haryx-api (Go) ─▶ Postgres 18     │
                 │        ── haryx.wecoap.com/platform/ ▶ static SPA      (haryx/deploy)  │
                 │        ── haryx.wecoap.com/* ───────▶ GitHub Pages (marketing)        │
                 │        nightly pg_dump of both databases, copied offsite in VN         │
                 └──────────────────────────────────────────────────────────────────────┘
```

- `/api/` and `/platform/` share an origin, so there is no CORS. Keycloak is on its own host.
- The apps still never hold a HaryX token or the sync key. The CLI does.

### Keycloak realm `wecoap`

The realm is shared by the WeCoAp consumer products. These rules keep products from reaching
each other:

- **Per-product permissions are client roles** (`haryx-api`'s roles, AR Onion's own), never realm
  roles. Every product's API checks `aud` for its own client, so a token issued for HaryX can't
  call AR Onion's API, and the other way round.
- **The realm's Terms and Privacy are the WeCoAp umbrella.** Each product asks for its own purposes
  and stores them itself, as HaryX does with `sync` and `machines`. A product never adds realm-wide
  consent attributes for its own purposes.
- **Realm-admin belongs to the owner alone.** Products get no Keycloak admin access: no service
  account and no secret (A7). A product acts on a user's account only with that user's own token,
  through Keycloak's account API.
- **One theme, branded per product if needed**: `wecoap` is the realm default, and a client can set
  its own login theme.
- **Client ids are prefixed with the product** (`haryx-*`, `aronion-*`), so the admin console and
  the event log say which product a session or consent belongs to.

The HaryX setup:

- Theme `wecoap` (jar from `../wecoap-sso`), with `configure-realm.mjs` from that repo (locales,
  consent attributes, events).
- **Clients**:
  - `haryx-cli`: public, PKCE S256, redirect `http://127.0.0.1/*` (loopback, any port; Phase 0
    checks this), device authorization grant on (for `--no-browser` over SSH), `offline_access`
    scope so a machine stays signed in past SSO idle limits.
  - `haryx-platform`: public, PKCE S256, redirect `https://haryx.wecoap.com/platform/*`, web
    origin `https://haryx.wecoap.com`, normal (not offline) sessions. Its token carries the
    `account` client's `manage-account` and `view-profile`, so the signed-in user can end their
    own machines' sessions through Keycloak's account API (A7).
  - **No admin service account** (A7). HaryX holds no Keycloak secret.
  - An **audience mapper** puts `haryx-api` in `aud` for both public clients, and an
    `identity_provider` mapper tells `whoami` which method was used.
- **Identity providers**: `github` and `google` (built in), `apple` (community extension
  `apple-identity-provider-keycloak`, pinned; Phase 0 checks it on Keycloak 26.8.2. Its jar is
  mounted next to the theme jar in `wecoap-sso/deploy/compose.yaml`, which restarts Keycloak). Email +
  password with *Verify email* on, and brute-force detection on.
- **SMTP**: `mail.wecoap.com:587`, STARTTLS, authenticated, from `no-reply@wecoap.com`, with
  reply-to and envelope sender on `wecoap.com`.
- **Production is set up by hand** by the owner, from the checklist in `deploy/README.md`
  ("Realm, by hand"). `deploy/keycloak/configure-local.mjs` sets up the same clients in a local
  Keycloak for tests, refuses anything but `127.0.0.1`, and changes with the checklist.

### HaryX API (`server/api/`, Go)

- Auth middleware:
  - Verifies the Bearer JWT against the realm JWKS (`github.com/coreos/go-oidc/v3`): `iss`,
    `aud ∋ haryx-api`, `azp ∈ {haryx-cli, haryx-platform}`, `exp`.
  - `sub` is the user id and `sid` is the session.
  - Refuses any `sid` in `revoked_sessions`. This closes the ≤5-minute window an access token has
    after a web sign-out.
- Everything is scoped by `sub`, in every query. There is no RLS to lean on, so the isolation tests
  below are the gate.

| Endpoint | Does | Replaces |
|---|---|---|
| `GET /api/v1/sync/rows` | List the caller's rows | PostgREST select |
| `POST /api/v1/sync/rows` | Insert `{kind,key,body}`. 409 if the row exists | `Remote::insert` → `None` |
| `PUT /api/v1/sync/rows/{kind}/{key}` | Update; `If-Match: <version>`, 412 if it moved; version +1 | `Remote::update` |
| `DELETE /api/v1/sync/rows/{kind}/{key}` · `DELETE /api/v1/sync/rows` | Delete one row / all rows | `delete`, `delete_all` |
| `PUT/GET/DELETE /api/v1/profiles/{hmac}` · `GET /api/v1/profiles` | Profile files: 10 MB each, 10 per user (as today's bucket) | Storage bucket |
| `GET /api/v1/consents` · `PUT /api/v1/consents/{purpose}` `{version}` · `DELETE …` | Read, grant, withdraw `sync` / `machines`. Withdrawal deletes that purpose's data in the same transaction | — |
| `PUT /api/v1/machines/self` | Register or heartbeat `{machine_id,name,os,version}`; needs the `machines` consent | — |
| `GET /api/v1/machines` · `PATCH /api/v1/machines/{id}` `{name}` | List, rename | — |
| `DELETE /api/v1/machines/{id}/session` | Sign out, step 2. The web has already ended the machine's session with the user's own token (`DELETE /realms/wecoap/account/sessions/{sid}`); the API checks the `sid` is one of the caller's machines, drops the machine and puts the `sid` into `revoked_sessions` | — |
| `GET /api/v1/me/export` | Data-subject access. There is no `DELETE /api/v1/me`: deleting the account is WeCoAp SSO's, on its account page (A10) | `delete-account` function |
| `GET /api/v1/health` | Liveness, no auth | `ping()` |

- Sync and profile endpoints answer **403 `consent_required`** without the `sync` consent.
- Rows keep today's limits: `kind` in the allowed set (now including `usage`), body < 256 KB.
- A per-user rate limit (nginx `limit_req` on `/api/`) replaces the abuse limits Supabase gave us.

**Schema** (`server/api/migrations/`, plain SQL, embedded, applied at start):

```sql
create table users (user_id uuid primary key, first_seen timestamptz,
  last_seen timestamptz);                -- touched per request, expired after 12 months (A10)
-- every user_id below: references users on delete cascade
create table sync_rows (user_id uuid, kind text, key text, body text, version bigint,
  updated_at timestamptz, primary key (user_id, kind, key));          -- same checks as today
create table profile_files (user_id uuid, name_hmac text, body bytea, updated_at timestamptz,
  primary key (user_id, name_hmac));
create table consents (user_id uuid, purpose text, version text, granted_at timestamptz,
  primary key (user_id, purpose));                                     -- current state
create table consent_events (id bigserial primary key, user_id uuid, purpose text,
  action text check (action in ('grant','withdraw')), version text, at timestamptz,
  source text);                                                        -- append-only proof
create table machines (user_id uuid, machine_id uuid, name text, os text, version text,
  session_id text, last_seen timestamptz, primary key (user_id, machine_id));
create table revoked_sessions (session_id text primary key, until timestamptz);
```

`consent_events` survive the deletion of the data they cover, as proof of the withdrawal. They are
deleted only with the user, when the `users` row expires (A10). Retention is stated in the
privacy policy.

### CLI (`cli/`)

The Supabase-specific code is confined to three files: `account/client.rs` (auth),
`sync/remote.rs` (PostgREST) and `sync/profiles.rs` (Storage). Encryption, merging, state and the
loopback/PKCE pieces stay.

- **`account/client.rs` → OIDC**:
  - Discovery from `$issuer/.well-known/openid-configuration`.
  - `authorize` with `kc_idp_hint=github|google|apple`, or no hint for email.
  - Code exchange, refresh, and logout as `revoke` of the refresh token.
  - The **device authorization grant** for `--no-browser`.
  - Keeps the Lessons rule: only Keycloak's own JSON `invalid_grant` means signed out, and every
    other status is a network error.
- **`--provider github|google|apple|email`**. With no flag, the WeCoAp page offers every method.
- **Session** gains `sid` and the issuer, and still drops anything it doesn't need. Keycloak tokens
  are larger than Supabase's. Phase 0 measured them: the full session is 2,396 bytes, over the
  Keychain's 1,968-byte limit (`MAX_KEYCHAIN_JSON`), so the compact form (1,013 bytes) is what's
  stored, as today.
- **`sync/remote.rs`**: an `Api` implementation of the existing `Remote` trait replaces
  `Postgrest`. `profiles.rs` points at `/api/v1/profiles`.
- **New verbs**:
  - `haryx account consents [--json]`
  - `haryx account consent grant|withdraw sync|machines [--yes]`
  - `haryx account export [PATH]`
- `haryx sync` heartbeats the machine when the `machines` consent is on. A local `machine_id`
  (random UUID) lives in the store.
- **Usage sync**: a new kind `usage`, one row per (machine, month), keyed by
  `row_key("usage:<machine_id>:<yyyy-mm>")`. Each machine writes only its own rows, so there is no
  merge, and other machines and the web read them.
- `HARYX_SUPABASE_URL` / `HARYX_SUPABASE_KEY` → **`HARYX_SSO_ISSUER`**, **`HARYX_API_URL`**,
  compiled in with env overrides for dev and tests.
- **Contracts** (`packages/contracts/haryx/`): update `whoami.json` (provider now includes `apple`
  and `email`; adds `consents`), and add `consents.json`, `machines.json`, `usage-sync.json`.

### macOS app and Windows app

The same changes on both, as `CLAUDE.md` requires:
- **Sign-in menu**: GitHub, Google, Apple, Email.
- **Consent sheets** before the first sync and before the machine toggle. They name the policy
  version, link to it, and offer Withdraw in the HaryX Account pane.
- **"Show this machine on HaryX web"** toggle in the HaryX Account pane.
- **"Open HaryX on the web"** link.
- **Remote sign-out notice**: "Signed out of HaryX from the web".
- **Usage pane copy**: "It stays on this machine" becomes conditional on the Sync consent.
- **Windows**: `consent`, `consents` and `export` join `ALLOWED_VERBS` under `account`.

### Web platform (`apps/platform/`)

- **Stack**: Vite + Svelte 5 (the Windows app's version), `base: '/platform/'`.
- **Auth**: `keycloak-js` (PKCE, tokens in memory, silent refresh).
- **Encryption**: `age-encryption` (typage) to unwrap the keyring and open rows. The row key is
  rebuilt in WebCrypto: SHA-256(context ‖ secret), then HMAC.
- **Shared code**: imports the Windows app's mascot renderer and formatting helpers through a Vite
  alias. They become a package only if a third consumer appears.
- **Sections**:
  - Machines: list, rename, sign out (with confirm), "this is you".
  - Accounts: alias, email, priority, mascot, which machines are signed in.
  - Usage: per account per day, all machines combined, with 5h/7d readings.
  - Profiles: list, download.
  - Privacy center: consents with their history, grant and withdraw, export, and a link to the
    WeCoAp account console for password, linked identities, sessions and deleting the account
    (A10).
- **Every section's states**: loading (skeleton), empty, error, signed out, locked, consent
  missing, success.
- **Trust boundary**:
  - A strict CSP (`default-src 'self'`; `connect-src 'self' https://sso.wecoap.com`), no
    third-party scripts, and Subresource Integrity on the build.
  - The passphrase and the key are never written to storage.
  - Locks after 15 minutes idle.
- **Verified** with a real browser at 375, 640, 768, 1024 and 1280 px.

### Deploy (`deploy/`)

Keycloak is already deployed from `wecoap-sso/deploy/` and is not redeployed here. HaryX's
`deploy/` follows the same aaPanel pattern (a Compose project, published on loopback only, with
aaPanel's site doing nginx and TLS):

- **`compose.yaml`**: `postgres:18-alpine` (database `haryx`, its own instance, not Keycloak's),
  `haryx-api` on `127.0.0.1:<port>`, and a `backup` job: nightly `pg_dump` of `haryx`, 14 days
  kept, copied offsite in Vietnam.
- **`nginx/haryx.wecoap.com.conf`**: the aaPanel site's config, kept in the repo: the current
  Pages proxy, `/api/` (with `limit_req`) and `/platform/` (static, SPA fallback to `index.html`).
- **TLS**: aaPanel's Let's Encrypt for `haryx.wecoap.com`, plus HSTS. A Cloudflare Origin
  certificate is not trusted by browsers once the proxy is off, so the certificate goes in first.
- **DNS switch for `haryx.wecoap.com`** (`sso.wecoap.com` is done), in order:
  1. Issue the certificate with the record still proxied. HTTP-01 reaches the origin through
     Cloudflare.
  2. Check the origin directly: `curl --resolve haryx.wecoap.com:443:<server-ip> …`.
  3. Flip the record to DNS-only.
  4. Check again that `dig` returns the VN server's IP, not Cloudflare's.
- **`.env.example`** and **`gen-secrets.sh`**:
  - Generated: the `haryx` DB password. HaryX holds no Keycloak secret (A7).
  - Hand-filled, named when missing: the GitHub, Google and Apple client secrets, the Apple key
    (team id, key id, `.p8`), and the SMTP user and password for `no-reply@wecoap.com`.
- **`release-platform.sh`**: builds `apps/platform` and copies `dist/` to the server.
- A **restore drill** is part of the release gate: a backup that has never been restored isn't a
  backup.

## Law 91/2025 compliance checklist

I'm not a lawyer. This lists what the code and docs must provide, and the legal text needs review.

- [ ] **Consent per purpose**:
  - Terms and Privacy at sign-up (done in `wecoap-sso`), Sync and Machine registry in HaryX.
  - Each one explicit, unticked by default, versioned, provable (`consent_events` and Keycloak
    events), and as easy to withdraw as to give.
- [ ] **Notice**: the `wecoap-sso` privacy policy (vi/en) covers HaryX's purposes, data categories,
  retention, the server location (Vietnam), processors (GitHub/Google/Apple as identity providers
  the user chooses), and the data-subject rights below.
- [ ] **Data-subject rights**:
  - Access and export: `/me/export`.
  - Correction: the Keycloak account console.
  - Erasure: the WeCoAp account page deletes the account, and HaryX erases that user's data when
    their `users` row expires, within 12 months (A10); withdrawal per purpose erases HaryX data
    and keeps the account.
  - Restriction: withdrawal.
  - Each one reachable from the web and the CLI.
- [ ] **Data residency**: all storage and backups in Vietnam, and mail through the self-hosted
  `mail.wecoap.com`. `sso.wecoap.com` and `haryx.wecoap.com` are DNS-only, so no foreign proxy
  terminates TLS. Check after the switch with `dig`.
- [ ] **Impact assessment dossier** for processing personal data: owner task, with this plan's data
  map as input.
- [ ] **Breach runbook**: who is told, and within what deadline the law sets. It goes in
  `docs/operations.md`.
- [ ] **Log retention**: nginx and Keycloak event logs, stated and enforced (events: 1,095 days, as
  `configure-realm.mjs` sets today; reconsider).

## Live realm, as found on 2026-11-26

Checked from outside, before HaryX registers anything:

- `sso.wecoap.com` is DNS-only on `160.187.0.66`, Let's Encrypt (valid to 2027-02-24), HSTS on.
- Realm `wecoap` answers discovery and offers the device grant. The `wecoap` login theme is
  served. No identity providers yet. Registration is open.
- `/health/ready` is not published, as intended (`wecoap-sso` keeps it on `:9000`).

**Prerequisites**, in `wecoap-sso` and on the server (owner), before any HaryX client goes live:

| # | Gap | Fix |
|---|---|---|
| P1 | **The sign-up form has no consent fields** (it asks for email, first and last name only), so `configure-realm.mjs` has not run on the live realm. Anyone can register now without the Law 91 Terms and Privacy consent. | Run `configure-realm.mjs` against the live realm, or turn registration off until it has run. Then sign up once and check that both consents are required and logged as events. |
| P2 | **The admin console and the `master` realm are public**: `/admin/master/console/` returns 200 to anyone. | In the aaPanel site for `sso.wecoap.com`, deny `/admin/` and `/realms/master/` except from an allowlisted IP or VPN. |
| P3 | The **bootstrap admin** from `.env` is meant for first start only. | Create a named permanent admin with OTP in `master`, delete the bootstrap one, and remove the bootstrap variables from `.env`. |
| P4 | **No backup of the Keycloak database**: `wecoap-sso/deploy/compose.yaml` has no backup job, and every WeCoAp ID lives there. | Add the same nightly `pg_dump` job as HaryX's, 14 days kept, copied offsite in VN, and do one restore drill. |

**Re-checked 2026-11-27**:

- **P1 is done.** The sign-up form now carries `terms_of_use` and `privacy_consent`. One real
  sign-up is still needed to see both consents required and logged as events.
- The realm now has the **`github` and `google`** identity providers. They are realm-wide, so
  AR Onion shares them. Task 2 therefore checks these two and doesn't create or overwrite them;
  it adds only `apple` and the `haryx-*` clients.
- **P2 is still open**: `/admin/master/console/` and `/realms/master/` both return 200.
- P3 and P4 can't be seen from outside. The owner confirms them.

Minor: the response carries two `Strict-Transport-Security` headers (Keycloak's realm header and
aaPanel's). Browsers use the first one. Dropping one is tidier, but not required.

## Phase 0: spike (before any build task)

Items 1–4 and 6 run against a local Keycloak **26.8.2** (the live version) started with
`npx keycloakify start-keycloak`, which `wecoap-sso` already uses. The live realm is shared, so
it is not used for experiments. Items 5 and 7 need the live host and run there, after P1–P4.

1. A `haryx-cli` client with redirect `http://127.0.0.1/*` accepts
   `http://127.0.0.1:<random>/callback/<nonce>` and refuses `localhost`, `127.0.0.1.evil.com`
   and so on.
2. Token sizes with `offline_access`: the stored session must fit under `MAX_KEYCHAIN_JSON`
   (1,968 bytes), or the access
   token is dropped from storage.
3. The admin API deletes **one** offline session by id, and the next refresh then fails with
   `invalid_grant`, leaving other machines alone.
4. The device authorization grant works end to end from `curl`.
5. The Apple extension loads on the pinned Keycloak version. On the existing Apple Developer
   account: a Services ID with return URL
   `https://sso.wecoap.com/realms/wecoap/broker/apple/endpoint` and a Sign in with Apple key.
   Apple's private-relay addresses only receive mail from registered senders, so `wecoap.com` /
   `no-reply@wecoap.com` is registered under *Sign in with Apple for Email Communication*.
6. `age-encryption` in Chrome and Safari unwraps a keyring written by the Rust CLI (scrypt
   log_n = 18), and we measure how long it takes. Above 5 s, show progress. It must not lower the
   work factor.
7. A Keycloak test email (verify-email and reset) sent through `mail.wecoap.com` arrives at Gmail
   and at an Apple private-relay address with SPF, DKIM and DMARC passing.

Any result that contradicts this plan stops the build and amends the plan.

### Phase 0 results (2026-11-27)

Run with throwaway scripts against local Keycloak 26.8.2 containers, a keyring written by the
CLI's own `SyncKey::wrap`, headless Chrome 153 and Safari 27.2. Items 5 and 7 are done only as
far as they can be without the live realm and the Apple account.

| # | Result |
|---|---|
| 1 | **Pass.** `http://127.0.0.1/*` accepts any port and path. It refuses `localhost`, `127.0.0.1.evil.com`, `127.0.0.1:p@evil.com`, `https://127.0.0.1`, `[::1]`, `127.0.0.2` and a smuggled query. A request without PKCE is refused. A full code flow to a random port with a `/callback/<nonce>` path returns tokens. |
| 2 | **Pass, with the limit corrected.** Access token 1,374 bytes, offline refresh token 730 (`typ: Offline`), ID token 1,083. With a 63-character email: full session 2,396 bytes, compact 1,013. The real limit is `MAX_KEYCHAIN_JSON` = 1,968, not 3,500, so the access token isn't stored and each CLI run refreshes first (one extra request; the access token lives 5 minutes anyway). `aud` = `["haryx-api","account"]`, `azp` = `haryx-cli`. |
| 3 | **Pass.** The token's `sid` is the offline session's id. `haryx-api-admin` deletes that one session (`DELETE /admin/realms/wecoap/sessions/{sid}?isOffline=true` → 204). That machine's refresh then fails with `invalid_grant` ("Offline user session not found"), while another machine keeps refreshing with the same `sid`. Revoking the refresh token (CLI sign-out) works the same way. The service account gets 403 on realm settings and on the client list. Its old access token still verifies by signature until it expires, which is why `revoked_sessions` stays. |
| 4 | **Pass, with PKCE.** `haryx-cli` requires S256, and Keycloak applies that to the device grant too: `auth/device` without `code_challenge` is refused. With it: `authorization_pending` before approval, then offline tokens. Interval 5 s, expiry 600 s. |
| 5 | **Partly done.** `apple-identity-provider` **1.17.0** (latest, built against 26.5.0, sha512 checked) loads on 26.8.2. The only warnings are Keycloak's "internal SPI" notices. With a test key, `kc_idp_hint=apple` redirects to `appleid.apple.com/auth/authorize` with `response_mode=form_post`, scope `openid name email`, and the broker endpoint as `redirect_uri`. **Still to do, by the owner**: the Services ID, the key and the private-relay sender registration on the Apple account, then one real sign-in. |
| 6 | **Pass in both browsers, but the work factor must be pinned.** `age-encryption` 0.3.1 unwraps the CLI's keyring and opens an X25519 row. WebCrypto rebuilds the keyring check and a row key exactly, and a wrong passphrase is refused. Two findings change the plan (below). |
| 7 | **Not run.** It needs the live realm's SMTP settings (task 2 against the live realm, after P2–P4). |

Unlock time by scrypt work factor (`log_n`):

| log_n | Memory | Rust (CLI) | Chrome 153 | Safari 27.2 |
|---|---|---|---|---|
| 18 | 256 MiB | ~0.5 s | 0.7 s | 0.6–1.5 s |
| 19 | 512 MiB | ~1.0 s | — | — |
| 20 | 1 GiB | — | 2.9 s | **12.7 s** |
| 21 | 2 GiB | — | refused | refused |

**Amendments from Phase 0** (they replace what the plan said before):

- **A1, keyring work factor is pinned at 18.** The `age` crate tunes scrypt until wrapping takes
  about a second on the writing machine, with no upper limit: it wrote 18 and 19 on the same Mac
  minutes apart. `age-encryption` refuses anything above 20, and Safari needs 12.7 s at 20. So
  `SyncKey::wrap` calls `set_work_factor(18)`, and `unwrap` refuses a keyring above 20 with a
  clear message. This replaces "it must not lower the work factor": 18 is what this Mac already
  picked by itself, and the 12-character minimum passphrase stays. New task 10a.
- **A2, the web unlock runs scrypt in a Web Worker** with a progress state. `age-encryption`'s
  scrypt is synchronous, so on the main thread the page freezes for the whole unlock. Task 18.
- **A3, any Admin API client needs the `kc-admin-api` client scope.** Keycloak 26.8.2 checks the
  Admin API audience; without the scope every admin call returns 401 ("Expected audience not
  available in the token"). Moot for HaryX since A7, kept for whoever adds an admin client.
- **A4, the CLI's device grant sends PKCE**: `code_challenge` on `auth/device` and
  `code_verifier` when polling. Task 9.
- **A5, `whoami` treats a missing `identity_provider` claim as `email`.** Password sign-ins carry no
  broker session note. Task 9.
- **A6, the Apple extension is pinned to 1.17.0** with its sha512, mounted in
  `wecoap-sso/deploy/compose.yaml`. Its "internal SPI" warnings make it part of the upgrade
  rehearsal for every Keycloak bump.
- **A7, no Keycloak admin access for HaryX** (owner, 2026-11-27, after the task 2 security
  review). On 26.8.2 a `manage-users` service account can't make itself realm-admin, but it can
  reset the password of any user in the shared realm, AR Onion's included. Tested instead: the
  user's own `haryx-platform` token, with `manage-account`, lists their sessions and ends one CLI
  machine's offline session through the account API, while their other machines keep working.
  So the web does step 1 of a sign-out itself, and the API only revokes the `sid` (task 8).
- **A8, production realm by hand.** The owner configures the live realm in the admin console from
  `deploy/README.md`; scripts only ever run against a local Keycloak.
- **A9, `trustEmail` off on every identity provider** (security review). With it on, Keycloak's
  first-broker-login links a social sign-in to an existing account with the same email without
  that account proving it owns the address.
- **A10, deleting the account belongs to WeCoAp SSO** (owner, 2026-11-27). There is no
  `DELETE /api/v1/me` and no `haryx account delete`. The web's Privacy center and the apps link to
  the WeCoAp account page, whose *Delete account* needs the realm's *Delete Account* required
  action and the `account` role `delete-account` in the default roles (`deploy/README.md`). That
  deletes the WeCoAp ID for every WeCoAp product, AR Onion included. Erasing only HaryX data is
  withdrawing both consents.
  - **HaryX is never told that a WeCoAp ID was deleted.** The realm's only event listener is
    `jboss-logging`, and HaryX has no admin access to read `DELETE_ACCOUNT` events. So **HaryX
    expires inactive users** (owner, 2026-11-27, over an SSO webhook): like every WeCoAp product,
    it keeps its own `users` table. Each authenticated request creates or touches the caller's
    row; every table's `user_id` references it `on delete cascade`; and a user with no request
    for **12 months** is deleted, everything of theirs with it, `consent_events` included. A
    deleted WeCoAp ID can make no request, so it is erased within 12 months.
  - What the user sees after 12 months idle: local data is untouched; the server copy is gone, so
    the next sync asks for the Sync consent again and uploads afresh.
  - The 12 months is `userRetention` in `server/api/cmd/haryx-api`, and the privacy policy
    states it. The policy also says that deleting the WeCoAp ID erases HaryX data within it.

## Tasks

Tags: dependency, then model. Each task is one commit in one area.

### Phase 1: infrastructure and contracts

| # | Task | Tag | Test |
|---|---|---|---|
| 1 | `deploy/`: compose (`haryx-api`, Postgres 18, backup job), the aaPanel site config for `haryx.wecoap.com` (Pages proxy included) with HSTS, the DNS-switch runbook, `.env.example`, `gen-secrets.sh` | independent · opus | `docker compose config`; stack up locally; `curl` the Pages proxy and `/api/v1/health`; after the switch, `dig` shows `160.187.0.66` and the certificate chain is Let's Encrypt |
| 2 | Realm, **by hand in production** (owner, `deploy/README.md` → "Realm, by hand"); `deploy/keycloak/configure-local.mjs` + `test-local.sh` for a local copy | independent · opus | Rerun twice against local Keycloak 26.8.2 (idempotent); a manual PKCE sign-in per method; then once against the live realm, after P1–P4 |
| 3 | `wecoap-sso`: privacy policy and terms cover HaryX's purposes; version bump | independent · sonnet | Its `check-layout.mjs`; legal review (owner) |
| 4 | Contracts: `whoami.json` (updated), `consents.json`, `machines.json`, `usage-sync.json` | independent · sonnet | Fixture decode tests in Rust, Swift and TS |

### Phase 2: HaryX API

| # | Task | Tag | Test |
|---|---|---|---|
| 5 | `server/api` skeleton: config, JWKS verification, revoked-session check, pgx, embedded migrations, health | independent · opus | A table test per claim: wrong `iss`, `aud`, `azp`, expired, revoked `sid`, `alg=none`, unknown `kid` |
| 6 | Sync rows and profile files with today's versioned semantics and limits | depends 5 · opus | Ports of the pgTAP checks as Go integration tests against compose Postgres: user A never reads or writes user B; 409/412 paths; size and count caps |
| 7 | Consents: grant, withdraw with deletion in one transaction, `consent_required` gating | depends 5, 6 · opus | Withdrawing sync leaves zero rows and profiles and one `withdraw` event; gated endpoints answer 403 |
| 8 | Machines, remote sign-out (the web ends the session with the user's token, the API revokes the `sid`), export | depends 5, 7 · opus | A `sid` that is not one of the caller's machines is refused; sign-out revokes only that `sid`; export holds every table |

### Phase 3: CLI

| # | Task | Tag | Test |
|---|---|---|---|
| 9 | OIDC sign-in: discovery, hints, device grant, revoke, session fields, new env vars | depends 4 · opus | `tiny_http` fake Keycloak; one test per status class (Lessons); stray-loopback and timeout tests kept |
| 10 | `Api` remote and API-backed profiles; Postgrest and Storage code removed | depends 4, 6 · sonnet | The existing two-machine engine tests pass unchanged on `Memory`; a fake-API test of 409/412 → `None` |
| 10a | `SyncKey::wrap` pins scrypt `log_n` = 18; `unwrap` refuses a keyring above 20 with a clear message (Phase 0, A1) | independent · opus | A unit test reads the wrapped header's `log_n` (18); a keyring at 21 is refused with the new message; the existing crypto tests pass |
| 11 | Consent, export and machine verbs; heartbeat in `sync`; the remote sign-out message | depends 7, 8, 9 · sonnet | `cli/crates/haryx/tests/account.rs`; `sync` with no consent says what to run |
| 12 | `usage` kind: per-machine monthly summaries | depends 10 · sonnet | Two-machine test: each machine writes only its own rows and reads both; no project paths in the plaintext before encryption |
| 13 | E2E: port `cli-e2e.sh` to the local compose stack (two CLI machines, real Keycloak, real API) | depends 2, 11, 12 · sonnet | The script itself |

### Phase 4: apps (13 must be green first)

| # | Task | Tag | Test |
|---|---|---|---|
| 14 | macOS: four sign-in methods, consent sheets, machine toggle, web link, notices, copy | depends 11, 12 · sonnet | `swift test` decode tests; manual run |
| 15 | Windows: the same, plus `ALLOWED_VERBS` | depends 11, 12 · sonnet | `npm run check`, `npm test`, `is_allowed` test; manual run |

Tasks 14 and 15 run in parallel.

### Phase 5: web platform

| # | Task | Tag | Test |
|---|---|---|---|
| 16 | `apps/platform` scaffold: keycloak-js, API client, layout, states, CSP | depends 4 · sonnet | `npm run check`, vitest |
| 17 | Machines and Privacy center | depends 8, 16 · sonnet | vitest; browser run against the local stack |
| 18 | Browser unlock: keyring unwrap, row keys, idle lock | depends 16 · opus | A fixture written by the Rust CLI (keyring, rows, HMAC keys) decrypts byte for byte in vitest; a wrong passphrase fails |
| 19 | Accounts, Usage and Profiles over unlocked data | depends 12, 18 · sonnet | vitest; browser run at 375–1280 px with stress data |

### Phase 6: cleanup, docs and review

| # | Task | Tag | Test |
|---|---|---|---|
| 20 | Remove `server/supabase/`, the keep-alive workflow and the Supabase env vars; update README ("What HaryX sends, and where"), `architecture.md`, `roadmap.md`, `CHANGELOG.md`; mark `haryx-account-plan.md` as superseded for the backend | depends 10–19 · haiku | `grep -ri supabase` finds only history |
| 21 | Merged-tree verification and security review: auth, consent and deletion, crypto in the browser, nginx | depends all · fable | Every check below on the combined tree |

## Verification

- **CLI**: `cargo fmt --check`, `cargo clippy`, `cargo test`.
- **macOS**: `swift build`, `swift test`.
- **Windows**: `npm run check`, `npm test`, `cargo test` in `src-tauri`.
- **API**: `go vet ./...`, `gofmt -l`, `go test ./...` against compose Postgres.
- **Web**: `npm run check`, vitest, a Playwright driver at 375, 640, 768, 1024 and 1280 px. It
  checks `scrollWidth <= innerWidth`, every state, and a clean console.
- **Stack**: `deploy/` up locally, then the ported `cli-e2e.sh`.
- **Release gate**: Phases 0–6 done, the restore drill passed, the privacy policy reviewed, the
  impact assessment filed. No tagged release ships sign-in before that.

## Surfaces

| Surface | Changes |
|---|---|
| CLI | Tasks 9–13 |
| macOS app | Task 14 |
| Windows app | Task 15 |
| Web platform (new) | Tasks 16–19 |

No surface is left out.

## Risks

- **We now operate the identity system.** Keycloak, Postgres, TLS and backups on one VN server are
  a single point of failure for sign-in. Signed-out and offline machines keep every local feature,
  as today. A Keycloak upgrade has to be rehearsed against the theme jar's version range.
- **Self-written authorization.** RLS is gone, so isolation lives in every query. Task 6's A/B
  tests and task 21's review are the guard.
- **The browser becomes a key holder.** A compromised `/platform/` build could take passphrases.
  CSP, no third-party scripts and SRI reduce that risk without removing it. The web unlock is
  optional, and the CLI and apps never need it.
- **Apple**: the extension is community-maintained. If Phase 0 fails, Apple ships after the rest.
- **No Cloudflare in front any more**: `haryx.wecoap.com` loses the CDN cache and DDoS shield, and
  the server's IP becomes public.
  - nginx caches the Pages responses and applies `limit_req` to `/api/` and to Keycloak's
    sign-in and token endpoints, and Keycloak's brute-force detection stays on.
  - A real attack would mean putting the proxy back and disclosing Cloudflare, which is a privacy
    policy change.
- **Email + password** brings password reset, brute force and deliverability into scope.
  Keycloak's brute-force detection and verified email cover the basics.
- **Long-lived offline sessions** on the CLI are what makes remote sign-out necessary. Revocation
  depends on task 8 working, so it gets its own test.

## Out of scope

- Remote commands to machines (sync now, switch account).
- Team view, paid tiers, MFA enforcement (Keycloak offers OTP to users who want it), mobile.
- Plaintext usage of any kind.
- Moving the marketing site off GitHub Pages.
- AR Onion's clients in the realm: AR Onion registers them itself, following the rules above.
  The checklist touches only `haryx-*` clients and the realm settings HaryX needs.
- Moving WeCoMed onto the realm, or adding "Sign in with WeCoAp" to it.
- A password-strength meter for the sync passphrase.

## Resolved questions

Settled with the owner on 2026-11-26:

- **Realm**: `sso.wecoap.com`, realm `wecoap`, shared with AR Onion.
- **Edge**: Cloudflare proxy off for `sso.wecoap.com` and `haryx.wecoap.com`.
- **Mail**: the self-hosted `mail.wecoap.com`, with a `no-reply@` account. DKIM is checked in Phase
  0 (item 7).
- **Apple**: the existing developer account.
- **The VN server** is big enough for the stack.
- **Keycloak** is live at `sso.wecoap.com` (2026-11-26), deployed from `wecoap-sso`, not from
  this repo.
- **Assumed**: `haryx.wecoap.com`'s origin is the same aaPanel server. Task 1's `curl --resolve`
  step confirms it before the DNS switch.

The plan assumes that the deploy steps (`deploy/`, the DNS switch, the restore drill) are run on
the server by the owner, or with SSH access given at that point. Everything before them runs
locally.
