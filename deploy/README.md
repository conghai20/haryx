# deploy/ — HaryX on the VN server

What runs where (plan: `docs/sso-platform-plan.md`):

| Piece | From | Where |
|---|---|---|
| WeCoAp SSO (Keycloak 26.8.2 + its Postgres) | `wecoap-sso/deploy/` | `sso.wecoap.com`, live |
| HaryX realm clients, Apple, SMTP | by hand, in the admin console | realm `wecoap`: see "Realm, by hand" |
| HaryX API + its Postgres + backups | `compose.yaml` | `127.0.0.1:18180`, aaPanel project `haryx` |
| Site config | `nginx/` | aaPanel site `haryx.wecoap.com` |
| Web platform | `apps/platform` → `release-platform.sh` | `/www/wwwroot/haryx.wecoap.com/platform/` |
| Marketing site | GitHub Pages (`conghai20.github.io/haryx/`) | proxied by the same site, as today |

The server is `160.187.0.66` (also `mail.wecoap.com` and `sso.wecoap.com`). Nothing that holds
personal data goes through Cloudflare.

## First deploy

1. **Secrets**: `./gen-secrets.sh`. It generates the database password and keeps it on rerun.
2. **Stack**: copy `compose.yaml`, `backup/` and `.env` to `/www/dk_project/haryx/`, then
   aaPanel → Docker → Compose → add `haryx`. `docker compose up -d`, then
   `curl -s http://127.0.0.1:18180/api/v1/health`.
3. **Site config**: copy `nginx/haryx-http.conf` and `nginx/haryx.wecoap.com.conf` to
   `/www/server/panel/vhost/nginx/`, and `nginx/haryx-headers.inc` to
   `/www/server/panel/vhost/nginx/haryx/`. `mkdir -p /www/server/nginx/proxy_cache_dir/haryx`,
   `nginx -t`, reload.
4. **Realm**: by hand, below. The realm prerequisites P1, P3 and P4 from the plan come first.

## Realm, by hand

In the admin console of `https://sso.wecoap.com`, realm **`wecoap`** (Keycloak 26.8.2). The code
depends on the names exactly as written: client ids, the audience `haryx-api`, the claim
`identity_provider`. HaryX holds no Keycloak secret and no admin account: the web signs a machine
out with the user's own token, through Keycloak's account API (plan A7).

### Realm settings

- **Login**: *Verify email* on.
- **Security defenses → Brute force detection**: *Lockout temporarily*, defaults.
- **Delete account** (plan A10, for every WeCoAp product): **Authentication → Required actions →
  Delete Account** enabled, then **Realm settings → User registration → Default roles → Assign
  role → filter by clients → `account`**: `delete-account`. The account page then shows *Delete
  account*. HaryX links there and has no delete of its own.
- **Email**: host `mail.wecoap.com`, port `587`, from `no-reply@wecoap.com`, from display name
  `WeCoAp`, reply to and envelope from `no-reply@wecoap.com`, *Enable StartTLS* on, *Enable SSL*
  off, *Authentication* on with the `no-reply@wecoap.com` mailbox. **Test connection.** If another
  WeCoAp product already set a different server, keep it and say so.

### Client `haryx-cli` (the CLI, and through it both apps)

1. **Clients → Create client**: OpenID Connect, client ID `haryx-cli`, name `HaryX`.
2. **Capability config**: *Client authentication* off, *Authorization* off. Flows: *Standard
   flow* on, *OAuth 2.0 Device Authorization Grant* on; *Direct access grants*, *Implicit flow*,
   *Service accounts* and *CIBA* off. *PKCE Method* `S256`.
3. **Login settings**: *Valid redirect URIs* `http://127.0.0.1/*` (literally that: any loopback
   port). Everything else empty, *Web origins* included.
4. **Client scopes** tab: `offline_access` stays **Optional**. Then open `haryx-cli-dedicated`:
   - **Add mapper → By configuration → Audience**: name `audience haryx-api`, *Included Custom
     Audience* `haryx-api`, *Add to access token* on, *Add to ID token* off.
   - **Add mapper → By configuration → User Session Note**: name `identity_provider`, *User
     Session Note* `identity_provider`, *Token Claim Name* `identity_provider`, type `String`,
     *Add to ID token* and *Add to access token* on.
   - **Scope** tab: *Full scope allowed* **off**.
5. **Advanced → Advanced settings**: *Proof Key for Code Exchange Code Challenge Method* `S256`, if
   step 2 did not already show it.

### Client `haryx-platform` (the web, `/platform/`)

1. **Create client**: OpenID Connect, client ID `haryx-platform`, name `HaryX on the web`.
2. **Capability config**: *Client authentication* off. *Standard flow* on; every other flow off,
   *Device Authorization Grant* included. *PKCE Method* `S256`.
3. **Login settings**: *Root URL* `https://haryx.wecoap.com`, *Home URL* `/platform/`, *Valid
   redirect URIs* `https://haryx.wecoap.com/platform/*`, *Valid post logout redirect URIs*
   `https://haryx.wecoap.com/platform/*`, *Web origins* `https://haryx.wecoap.com`.
4. **Client scopes** tab: **remove `offline_access`**. The browser never holds an offline token.
5. `haryx-platform-dedicated`: the same two mappers as `haryx-cli`. **Scope** tab: *Full scope
   allowed* **off**, then **Assign role → filter by clients → `account`**: `manage-account` and
   `view-profile`. This is what lets the signed-in user end their own machines' sessions, and
   nothing else: only their own account.

### Identity providers

- **`github`, `google`**: already there. Leave them, except *Trust email*: **off** on both. With it
  on, a first sign-in links to any existing account with that email without that account proving
  it owns the address.
- **`apple`**: needs the extension in Keycloak first (plan A6): add
  `apple-identity-provider-1.17.0.jar` from
  `https://github.com/klausbetz/apple-identity-provider-keycloak/releases/tag/1.17.0` (check its
  `.sha512`) next to the theme jar in `wecoap-sso/deploy/compose.yaml`, and restart Keycloak.
  Then **Add provider → Apple**:
  - *Client ID*: the Services ID. *Team ID*, *Key ID*: from the Sign in with Apple key.
  - *Client Secret*: the body of the key's `.p8` (the lines between `BEGIN` and `END`).
  - *Default scopes* `name email`. *Trust email* **off**.
  - In Apple Developer: the Services ID's return URL is
    `https://sso.wecoap.com/realms/wecoap/broker/apple/endpoint`, domain `sso.wecoap.com`. Under
    *Sign in with Apple for Email Communication*, register `wecoap.com` and `no-reply@wecoap.com`,
    or mail never reaches private-relay addresses.

### Test realm `wecoap-test`, for the end-to-end test

A second realm on the same Keycloak, so the E2E (plan task 13) signs in for real without touching
`wecoap` or its users. By hand, like the rest:

1. **Create realm** `wecoap-test`. **Realm settings → Login**: *User registration* off, *Verify
   email* **off** (no mail is sent; the users below are verified by hand). **Security defenses →
   Brute force detection**: **off**, so repeated scripted sign-ins never lock a user. Themes: leave
   the default; the script reads Keycloak's own login form.
2. **Realm settings → Action (top right) → Partial import**: the file
   `deploy/keycloak/wecoap-test-import.json`, *If a resource exists*: **Fail**. It adds the two
   clients as above (mappers and scopes included; `haryx-platform` also allows the dev server
   `http://127.0.0.1:5173`) and the users `e2e-a` and `e2e-b` (verified, named, no required
   actions, **no password**). Identity providers: none; social sign-in is checked by hand on
   `wecoap`.
3. **What the import cannot carry**, by hand:
   - **Clients → `haryx-platform` → Client scopes → `haryx-platform-dedicated` → Scope → Assign
     role → filter by clients → `account`**: `manage-account` and `view-profile`.
   - **Users → `e2e-a` → Credentials → Set password**, *Temporary* off; the same for `e2e-b`.
4. Put the details in **`deploy/.env.e2e`** (never committed), not in chat:

   ```
   HARYX_E2E_ISSUER=https://sso.wecoap.com/realms/wecoap-test
   HARYX_E2E_USER_A=e2e-a
   HARYX_E2E_PASSWORD_A=…
   HARYX_E2E_USER_B=e2e-b
   HARYX_E2E_PASSWORD_B=…
   ```

### The same, locally

`deploy/keycloak/test-local.sh` sets these clients up in a throwaway Keycloak 26.8.2 with
`configure-local.mjs`, runs it twice with a hand edit in between, and signs in through each
client, including a sign-out of one machine from the web (`APPLE_JAR=…` adds Apple). The script
refuses anything but `http://127.0.0.1:<port>`. Change it together with this checklist.

### Check from outside (no admin needed)

```bash
B=https://sso.wecoap.com/realms/wecoap/protocol/openid-connect
P='code_challenge=abcdefghijabcdefghijabcdefghijabcdefghij123&code_challenge_method=S256'
# 200: loopback accepted.  400: localhost refused.
curl -s -o /dev/null -w '%{http_code}\n' "$B/auth?client_id=haryx-cli&response_type=code&scope=openid%20offline_access&redirect_uri=http://127.0.0.1:5555/callback/x&$P"
curl -s -o /dev/null -w '%{http_code}\n' "$B/auth?client_id=haryx-cli&response_type=code&scope=openid&redirect_uri=http://localhost:5555/callback/x&$P"
# device grant answers with a user_code
curl -s "$B/auth/device" -d client_id=haryx-cli -d 'scope=openid offline_access' -d code_challenge=abcdefghijabcdefghijabcdefghijabcdefghij123 -d code_challenge_method=S256
# the platform refuses offline_access: the Location carries error=invalid_scope
curl -s -o /dev/null -w '%{redirect_url}\n' "$B/auth?client_id=haryx-platform&response_type=code&scope=openid%20offline_access&redirect_uri=https://haryx.wecoap.com/platform/cb&$P"
```

## DNS switch for haryx.wecoap.com

`sso.wecoap.com` is already DNS-only. `haryx.wecoap.com` is still proxied by Cloudflare. A
Cloudflare Origin certificate is not trusted by browsers, so the order matters:

1. **Certificate first.** aaPanel → site → SSL → Let's Encrypt, with the record still proxied
   (HTTP-01 reaches the origin through Cloudflare). On 2026-11-27 the origin already served a
   Let's Encrypt certificate valid to 2027-02-24, so this step was done.
2. **Check the origin directly**, past Cloudflare:

   ```bash
   curl -sI --resolve haryx.wecoap.com:443:160.187.0.66 https://haryx.wecoap.com/ | head -3
   curl -s  --resolve haryx.wecoap.com:443:160.187.0.66 https://haryx.wecoap.com/api/v1/health
   echo | openssl s_client -connect 160.187.0.66:443 -servername haryx.wecoap.com 2>/dev/null \
     | openssl x509 -noout -issuer -enddate     # issuer: Let's Encrypt
   ```

3. **Flip** the record to DNS-only (grey cloud) in Cloudflare.
4. **Check** that `dig +short haryx.wecoap.com` prints `160.187.0.66` and nothing from Cloudflare,
   and run step 2 again without `--resolve`.

If it goes wrong, turn the proxy back on: the origin keeps serving either way.

## Backups

The `backup` container writes `haryx-<date>.dump` (`pg_dump -Fc`) once a day after 03:00
(Asia/Ho_Chi_Minh) into `HARYX_BACKUP_DIR`, and deletes dumps older than 14 days. A dump now:
`docker compose run --rm backup once`.

**Offsite, in Vietnam**: aaPanel → Cron → a daily Shell task after 04:00 that copies the folder to
the second VN location, for example `rsync -a --delete /www/dk_project/haryx/backups/ <host>:<dir>/`.
The target is the owner's choice. It must stay in Vietnam (Law 91: data residency).

Keycloak's own database is `wecoap-sso`'s to back up (plan P4).

### Restore drill — part of every release gate

A backup that has never been restored is not a backup. Into a scratch database, never `haryx`:

```bash
docker compose run --rm backup once
latest=$(ls -t backups/haryx-*.dump | head -1)
docker exec haryx-postgres createdb -U haryx haryx_restore
docker exec -i haryx-postgres pg_restore -U haryx -d haryx_restore --no-owner < "$latest"
for db in haryx haryx_restore; do
  docker exec haryx-postgres psql -U haryx -d $db -Atc \
    "select '$db', count(*) from information_schema.tables where table_schema = 'public'"
done
docker exec haryx-postgres dropdb -U haryx haryx_restore
```

Both lines must show the same table count. Note the date in the release notes.
