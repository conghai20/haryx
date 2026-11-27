# deploy/ — HaryX on the VN server

What runs where (plan: `docs/sso-platform-plan.md`):

| Piece | From | Where |
|---|---|---|
| WeCoAp SSO (Keycloak 26.8.2 + its Postgres) | `wecoap-sso/deploy/` | `sso.wecoap.com`, live |
| HaryX realm clients, Apple, SMTP | `keycloak/configure-haryx.mjs` | realm `wecoap` |
| HaryX API + its Postgres + backups | `compose.yaml` | `127.0.0.1:18180`, aaPanel project `haryx` |
| Site config | `nginx/` | aaPanel site `haryx.wecoap.com` |
| Web platform | `apps/platform` → `release-platform.sh` | `/www/wwwroot/haryx.wecoap.com/platform/` |
| Marketing site | GitHub Pages (`conghai20.github.io/haryx/`) | proxied by the same site, as today |

The server is `160.187.0.66` (also `mail.wecoap.com` and `sso.wecoap.com`). Nothing that holds
personal data goes through Cloudflare.

## First deploy

1. **Secrets**: `./gen-secrets.sh`, then fill what it names. Rerun it until it names nothing you
   need (Apple and SMTP can wait; the script skips them).
2. **Stack**: copy `compose.yaml`, `backup/` and `.env` to `/www/dk_project/haryx/`, then
   aaPanel → Docker → Compose → add `haryx`. `docker compose up -d`, then
   `curl -s http://127.0.0.1:18180/api/v1/health`.
3. **Site config**: copy `nginx/haryx-http.conf` and `nginx/haryx.wecoap.com.conf` to
   `/www/server/panel/vhost/nginx/`, and `nginx/haryx-headers.inc` to
   `/www/server/panel/vhost/nginx/haryx/`. `mkdir -p /www/server/nginx/proxy_cache_dir/haryx`,
   `nginx -t`, reload.
4. **Realm**: from any machine that reaches `sso.wecoap.com`, with `deploy/.env` loaded:

   ```bash
   set -a; . deploy/.env; set +a
   KC_ADMIN_USER=… KC_ADMIN_PASSWORD=… node deploy/keycloak/configure-haryx.mjs
   ```

   It needs the realm prerequisites P1–P4 from the plan done first. The Apple provider also needs
   the extension jar mounted in `wecoap-sso/deploy/compose.yaml` (plan A6).

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
