# HaryX sign-in and encrypted sync across machines: plan

Status: in progress (2026-11-25).
- Phase 0: done (spike locally, GitHub end to end on the hosted project).
- Phase 1: tasks 2–4 built on `feat/haryx-account`: `haryx login`, `logout` and `whoami`. They were
  verified against the hosted project with the real Keychain.
- Remaining: task 1 (Google client), task 5 (`account delete`), then Phases 2–4.

## Decisions

Taken with the owner on 2026-11-25:

| Topic | Decision |
|---|---|
| Purpose | A HaryX account exists to **sync across machines**. Team view, paid tiers and web/mobile come later. |
| Sign-in | **GitHub and Google** first. The email code and email + password methods follow once HaryX owns a domain to send email from. |
| Backend | **Supabase Free**, kept awake by a daily keep-alive job. No HaryX server is written. The Go `server/` stays for team view. |
| Surfaces | CLI, macOS app and Windows app, all in the first release. |
| Encryption | **End to end, with a sync passphrase** the user sets. Supabase stores only ciphertext. |
| Sync scope | HaryX settings, app preferences, Claude account metadata, setup profiles. |
| Claude logins on a new machine | **Signed in on that machine.** The new machine lists the accounts it learned from sync, and the user signs each one in with `haryx ai login`. Browser-login credentials are never copied between machines. |

Defaults this plan takes (change any of them before building):
- **Signing in is optional.** Signed out, HaryX behaves exactly as today and sends nothing.
- **Local-first.** Each machine's store is the source of truth; the server is a relay. A paused or
  lost Supabase project delays sync and loses nothing.
- **No conversation content leaves the device**, not even encrypted. Setup profiles sync without
  history.
- **API-key and setup-token accounts** sync their credential, encrypted, because no login is
  involved. Drop this (task 13) to keep every credential on its own machine.

## The flow, start to finish

1. **First machine (Mac).**
   - The user opens HaryX › HaryX Account and clicks *Continue with GitHub*. The browser opens,
     they approve, and the browser returns them to HaryX.
   - HaryX asks for a **sync passphrase**, at least 12 characters, with a warning: "HaryX cannot
     recover it. Any machine that is still unlocked can set a new one."
   - The first sync uploads settings, preferences and account metadata, all encrypted.
2. **Second machine (Windows).**
   - The user installs HaryX, clicks *Continue with GitHub* with the same account, and enters the
     passphrase.
   - Settings and preferences arrive.
   - Accounts shows **"3 accounts on your other machines"**, each with **Sign in here**. That runs
     the normal Claude Code browser sign-in. Once it finishes, the account appears with its synced
     alias, priority, threshold and mascot.
3. **When something fails.**
   - **The browser signs in as a different Claude account** than the row expects: nothing is stored,
     and the message names both emails. The user switches account on claude.ai and tries again.
   - **Wrong passphrase:** "Can't unlock sync", with try again or reset.
   - **Backend paused or offline:** "Sync unavailable, will retry", and everything local keeps working.
   - **HaryX session refused:** "Signed out of HaryX", with sign in.
   - **Conflict:** the newer write wins, and what it overwrote is listed in Sync status.

## What already exists

- **`haryx login` is reserved for this.** `cli/crates/haryx/src/cli/mod.rs` lists it in `AI_ONLY`
  ("`haryx login` will sign in to a HaryX account"), and `top.rs` `moved_to_ai` answers it with a
  usage error today.
- **Claude sign-in.** `haryx ai login` (`haryx-core/src/switcher/login.rs`) runs Claude Code's own
  `claude auth login` in a throwaway staging profile, so the active login is untouched.
  `ensure_same_account` refuses a login that came back as a different account than the slot
  expects. "Sign in here" reuses all of it.
- **HTTP.** `ureq` 3 (rustls), already used by `oauth/`. Its tests use a `tiny_http` fake server.
- **Secret storage and encryption.**
  - macOS: `keychain.rs` (`/usr/bin/security`, silent across upgrades).
  - Elsewhere: `credentials/sealed.rs` (a 0600 file on Linux, DPAPI on Windows).
  - Locks: `locking.rs`.
  - Encryption: the `age` crate, already a dependency, with scrypt passphrase recipients.
- **Setup profiles.** `ai profile export/import` produce one portable file, with `--secrets` and
  `--no-history` (`ai-profile-plan.md`).
- **The apps.**
  - macOS runs the bundled `haryx` and reads streamed `--json` lines (as with `auto --json`).
  - Windows goes through `apps/windows/src-tauri/src/commands.rs`, whose `ALLOWED_VERBS` allowlist
    has no account verbs yet.
  - Rule kept: **the apps never hold a HaryX token or the sync key**. The CLI does.

## Commands

```
haryx login [--provider github|google] [--no-browser] [--json]   # loopback callback
haryx logout [--json]
haryx whoami [--json]                    # email, provider, user id, sync locked/unlocked; never a token
haryx account delete [--yes]             # deletes the HaryX account and every synced item

haryx sync [--json]                      # push local changes, pull remote ones
haryx sync status [--json]               # last sync, pending, overwritten items, other-machine accounts
haryx sync unlock [--passphrase-stdin]   # unlock this machine; the first machine sets the passphrase
haryx sync passphrase change             # re-wrap the key; needs an unlocked machine
haryx sync reset [--yes]                 # forgotten passphrase: wipe the server copy, re-upload from here
haryx sync forget EMAIL                  # drop an account from the "other machines" list everywhere
haryx sync profile push NAME | pull NAME | list
haryx sync prefs get [--json]            # app preferences, read by the apps
haryx sync prefs set                     # app preferences, JSON on stdin, written by the apps

haryx ai login EMAIL                     # new: EMAIL may be an account known only from sync
```

- `--provider` defaults to `github`. `--no-browser` prints the link (over SSH). `$BROWSER`, when set,
  is the program that opens it.
- `haryx login 3` is a usage error pointing at `haryx ai login 3`.
- `login --json` streams events:
  - `{"event":"browser","url":…}`: the URL, for when no browser opened.
  - `{"event":"signed_in","user":{…}}`
  - `{"event":"error","code":…,"message":…}`
- **`ai login EMAIL` for a synced-only account:**
  1. Makes a new slot.
  2. Runs the usual staging login.
  3. Checks that the identity matches EMAIL, using the same rule as a re-login.
  4. Applies the synced metadata.
  A mismatch stores nothing.
- Passphrases come only from stdin or a no-echo prompt, never from argv.
- `sync --json` reports what changed (`"changed":["settings","app-prefs",…]`), so the apps reload
  only that.
- Shapes go into `packages/contracts/haryx/` first: `whoami.json`, `login-events.json`,
  `sync-status.json`, `sync-result.json`, `app-prefs.json`.
- `logout`, `whoami`, `account` and `sync` are not upstream `cswap` verbs, so they collide with nothing.

## HaryX sign-in (Supabase Auth, `/auth/v1`)

**GitHub and Google, through PKCE and a loopback callback:**

1. The CLI makes a verifier and challenge, and binds `127.0.0.1:0`. It uses the literal IP, never
   `localhost`. Auth accepts any loopback port and path with no allow-list entry (Phase 0). The
   path is `/callback/<random nonce>`, so a page or process that merely finds the port cannot end
   the sign-in.
2. It opens the browser to `/authorize?provider=github|google&redirect_to=http://127.0.0.1:PORT/callback&code_challenge=…&code_challenge_method=s256`.
3. It accepts the redirect on that path; anything else gets a 404 and the wait goes on. Each
   connection has 2 s in total to send its request, so a silent or trickling socket cannot stall
   it. The browser's page is sent only after the exchange, in the site's design (DESIGN.md ›
   Sign-in Return Page): who signed in, or why it did not finish.
4. It POSTs `/token?grant_type=pkce` with `auth_code` and `code_verifier`.
5. It times out after 5 minutes.

Over SSH, where no browser can reach the loopback, it prints the URL and says so.

**Linking.** Supabase links GitHub and Google identities that share a verified email into one account.

**The stored session.**
- Kept: the access and refresh tokens, `expires_at`, `user.id`, `user.email`.
- Dropped: `provider_token`.
- Where:
  - macOS: Keychain service `haryx`, account `haryx-session`.
  - Linux and Windows: `<store>/account/session` through `sealed::seal`.
- **Renewal:** on use, when less than 60 s are left, under a `locking.rs` lock so an app and a
  terminal never renew at the same moment.
- A refused renewal means "signed out", with a notice. A network error leaves the session alone.

**Project URL and publishable key.** Compiled in. They are public by design; row-level security,
plus the end-to-end encryption, protect the data. `HARYX_SUPABASE_URL` and `HARYX_SUPABASE_KEY`
override them for a dev project and for tests.

## Encryption

- **The sync key** is an age X25519 identity, made on the first machine at first unlock.
- **Keyring.** The key is wrapped with the passphrase (age scrypt) and stored as the `keyring` row.
  Each machine keeps its unwrapped key in its own secret store (Keychain item `haryx-sync-key`, or
  `sealed`), so the passphrase is asked for once per machine.
- **Every other row and file** is age-encrypted to the key's recipient. The server sees only:
  the kind, an opaque key, the version, timestamps and the size.
- **Opaque keys.** An account row is keyed by HMAC-SHA256 of the lowercased account email, under a
  subkey derived from the sync key. The server can neither read emails nor guess them from a list of
  addresses. Adds the `hmac` crate (RustCrypto, pairs with `sha2`) rather than hand-written HMAC.
- **Changing the passphrase** re-wraps the same key, so nothing else is re-encrypted.
- **A forgotten passphrase.** Any machine still unlocked can set a new one. If none is, `sync reset`
  deletes the server copy and uploads again from the local store.
- **Strength.** At least 12 characters. The wrapped keyring sits on Supabase, so a weak passphrase
  could be brute-forced offline by anyone who obtained the database.

## What syncs

| Kind (row) | Contents | Merge |
|---|---|---|
| `keyring` | The wrapped sync key | Written once; re-wrapped on a passphrase change |
| `settings` | `haryx config` keys marked syncable. Terminal app, shell, paths and `sync.*` stay local; the list lives in code beside the settings schema. | Whole document, newest wins |
| `app-prefs` | Shared keys (theme, notification toggles, usage display), plus `macos.*` / `windows.*` sections that only the same platform applies. Keys are listed in `app-prefs.json`. | Per key, newest wins |
| `account` (one per Claude account) | Email, alias, enabled, threshold, priority, mascot species and colour. Never usage numbers, and never a browser-login credential. | Per account, newest wins |
| `credential` (API-key and setup-token accounts only) | The key or token | Per account, newest wins |
| profile file | A setup profile exported with `--secrets redact --plugins ref --no-history`, then encrypted whole. `--credentials` is refused. | Explicit push/pull by name; a pull only **imports**, and applying stays `ai profile apply` |

Stays local: browser-login credentials, folder mappings (they are paths), usage history, session data.

**Accounts on other machines.**
- An `account` row with no local slot shows as "on your other machines", with **Sign in here**.
- A row whose local slot exists applies its metadata to that slot.
- Removing an account on one machine removes it only there. `sync forget` removes its row
  everywhere.

**When sync runs.**
- `haryx sync` by hand.
- The apps run it at launch, 5 s after a synced change (debounced), and every 15 minutes.
- Every write carries the version it last saw. If the server's version moved, the client pulls,
  merges newest-wins and retries once. Overwritten values go to `sync status` and `haryx.log`.

## Backend: Supabase Free, in `server/supabase/`

- **`migrations/0001_sync.sql`**
  ```sql
  create table public.sync_rows (
    user_id    uuid not null default auth.uid() references auth.users on delete cascade,
    kind       text not null check (kind in
                 ('keyring','settings','app-prefs','account','credential')),
    key        text not null default '',          -- '' or an HMAC hex digest
    body       text not null check (length(body) < 262144),   -- base64 age ciphertext
    version    bigint not null default 1,
    updated_at timestamptz not null default now(),
    primary key (user_id, kind, key)
  );
  alter table public.sync_rows enable row level security;
  create policy own_rows on public.sync_rows
    for all using (user_id = auth.uid()) with check (user_id = auth.uid());

  create function public.ping() returns timestamptz language sql stable as $$ select now() $$;
  ```
- **Storage.** A private bucket `profiles`:
  - Object path `<uid>/<hmac(name)>`. The policy allows it only when the first folder is
    `auth.uid()`.
  - 10 MB per file and 3 files per user, enforced in the policy.
  - The free plan has 1 GB in all.
- **`functions/delete-account/`.** An Edge Function that deletes the caller's storage objects (they
  do not cascade) and then `auth.admin.deleteUser(uid)`. The secret key lives only in that
  function's environment.
- **`tests/`.** pgTAP tests (`supabase test db`), all security-critical:
  - user A can neither read nor write user B's rows or objects;
  - the anonymous key reaches nothing but `ping()`;
  - the size and count limits hold;
  - deleting an account removes everything.
- **`config.toml`** for the local stack (`supabase start`, Docker): the GitHub and Google providers,
  `site_url`, and token rotation with its 10 s reuse interval. Client secrets come in through
  `env(…)`.
- **Keep-alive.** `.github/workflows/keepalive.yml` calls `rpc/ping` with the publishable key once
  a day.
  - GitHub turns off scheduled workflows in a repository with no activity for 60 days. An active
    repo is fine; the release checklist notes it.
  - The apps' 15-minute syncs are activity too, once anyone uses sync.
- **Kept out of the repo:** the GitHub and Google client secrets. They live in the Supabase dashboard.

## Phase 0: spike

Run on 2026-11-25 against a local stack. Setup: Supabase CLI 3.1.0 through `npx`, Docker,
Auth (GoTrue) v2.197.0, and `site_url` set to the production value `https://conghai20.github.io/haryx/`.
Probed with `curl`, reading the stored redirect from `auth.flow_state`.

1. **Loopback redirect: no allow-list needed.**
   - Auth accepts any `http://127.0.0.1:<port>/…` and `http://[::1]:<port>/…`, even with
     `additional_redirect_urls = []`. This is RFC 8252 behaviour for native apps.
   - `localhost`, `127.0.0.1.evil.com` and `evil.com/127.0.0.1:1/…` all fall back to `site_url`.
   - So the CLI binds `127.0.0.1:0` and uses that literal IP, never `localhost`.
   - PKCE is what makes this safe: a code that reaches another local listener cannot be
     exchanged without the verifier.
2. **Session size fits the Keychain.** The full signup response is 2,834 bytes. The stored part
   (access and refresh tokens, `expires_at`, id, email) is 1,344 bytes, or 2,688 hex-encoded,
   under the 4,096-byte `security -i` limit. The access token is a 1,183-byte JWT (1-hour expiry)
   and the refresh token is 12 characters.
   - Rule: if a GitHub or Google profile ever makes the JWT push the value past 3,500 bytes, store
     without the access token rather than let `keychain.rs` fall back to argv.
3. **Renewal is forgiving for 10 s.** With `enable_refresh_token_rotation = true` and
   `refresh_token_reuse_interval = 10` (the defaults), renewing twice with the same old token
   returned the same new token. Two processes renewing together don't sign each other out. The
   `locking.rs` lock stays for renewals further apart.
4. **Unlocking takes about 0.6 s.** scrypt at age's work factor (log_n = 18, r = 8, p = 1) took
   0.56 s on an Apple M2 (0.26 s at 17, 0.13 s at 16). A machine three times slower stays under 2 s.
5. **GitHub works end to end on the hosted project** (`ygvcznwlugaxqqmlsssh`, 2026-11-25).
   - The flow: PKCE, a callback on a random port (`127.0.0.1:54849`) with no allow-list entry, then
     `/token?grant_type=pkce`.
   - The user comes back with a confirmed email.
   - Sizes: the session is 3,048 bytes and the access token 1,202 (1-hour expiry). The stored part
     is 1,359 bytes, or 2,718 hex-encoded, which fits `security -i`.
   - GitHub's profile adds 11 `user_metadata` keys but barely grows the token.
   - The response carries `provider_token`, which is dropped as planned.
   - **Google is still open**, waiting for the OAuth client (task 1).

## Tasks

Each task is tagged *independent* or *depends on N*.

### Phase 1: HaryX sign-in

| # | Task | Tag | Test |
|---|---|---|---|
| 1 | Hosted Supabase project and `server/supabase/config.toml`. A GitHub OAuth App and a Google OAuth client, both with callback `https://<ref>.supabase.co/auth/v1/callback`. Redo the Phase 0 loopback probe against the hosted project. | depends on Phase 0 | Manual PKCE sign-in with each provider. Both identities link to one email. |
| 2 | Contracts: `whoami.json`, `login-events.json` | independent | Fixture decode tests, Rust and Swift |
| 3 | `haryx-core/src/account/`: `client.rs` (`ureq` calls), `pkce.rs`, `loopback.rs`, `session.rs` (store, load, renew under lock) | depends on 2 | Integration tests against a `tiny_http` fake of `/auth/v1`: two processes renewing at once, refused renewal, timeout, stray loopback request |
| 4 | Take `login` out of `AI_ONLY`. Add `login`, `logout`, `whoami` and `account delete` as top-level commands, and to the help overview. | depends on 3 | `cli/crates/haryx/tests/account.rs`; update the `moved_to_ai("login")` test |
| 5 | `server/supabase/functions/delete-account` | depends on 1 | Local stack test |

### Phase 2: encrypted sync

| # | Task | Tag | Test |
|---|---|---|---|
| 6 | `migrations/0001_sync.sql`, `ping()`, bucket and policies, pgTAP tests, keep-alive workflow | depends on 1 | `supabase test db` in CI (Docker on `ubuntu-latest`) |
| 7 | Contracts: `sync-status.json`, `sync-result.json`, `app-prefs.json` | independent | Fixture decode tests |
| 8 | Mark each `haryx config` key syncable or local | independent | Unit test: every key is classified |
| 9 | `haryx-core/src/sync/crypto.rs`: key generation, wrap and unwrap, encrypt and decrypt, HMAC keys, local key storage | independent | Unit tests: round trip, wrong passphrase, key cached and reused, re-wrap |
| 10 | `haryx-core/src/sync/engine.rs`: local ↔ rows, versioned upsert, newest-wins merge, overwrite log. Kinds `settings`, `app-prefs`, `account`. | depends on 3, 6, 7, 8, 9 | Fake PostgREST; a two-machine test with two stores and one fake backend, covering a conflict |
| 11 | `sync`, `sync status`, `sync unlock`, `passphrase change`, `reset`, `forget`, `prefs get/set` | depends on 10 | CLI integration tests |

### Phase 3: Claude accounts on a new machine, and profiles

| # | Task | Tag | Test |
|---|---|---|---|
| 12 | `ai login EMAIL` for a synced-only account: new slot, staging login, `ensure_same_account`, then apply the metadata | depends on 10 | `cli/crates/haryx/tests/login.rs`: a fake `claude` that signs in as the right account, and one that signs in as the wrong account (nothing stored) |
| 13 | `credential` rows for API-key and setup-token accounts. Slot numbers are assigned locally on arrival. | depends on 10 | Two-machine test: the account arrives and works, and a browser-login account is never pushed |
| 14 | `sync profile push/pull/list`: reuse `ai profile export/import`, encrypt the whole file, refuse history and credentials | depends on 10 | CLI integration test against fake Storage |

### Phase 4: apps

| # | Task | Tag | Test |
|---|---|---|---|
| 15 | macOS: a **HaryX Account** pane under the HaryX sidebar group.<br>Signed out: Continue with GitHub / Google.<br>Signed in: email, unlock / set passphrase, Sync on/off, last sync, Sync now, overwritten items, Sign out, Delete account (confirm).<br>Accounts: an "on your other machines" section with Sign in here.<br>Sync runs at launch, on change and every 15 min.<br>Preferences go through `sync prefs` and reload on `changed`. | depends on 4, 11, 12 | `HaryXCore` decode tests; manual run on two machines |
| 16 | Windows: add `login`, `logout`, `whoami`, `account` and `sync` to `ALLOWED_VERBS`, and the same pane and Accounts section in Svelte | depends on 4, 11, 12 | `is_allowed` unit test; manual run |

Tasks 15 and 16 run in parallel.

**Release gate.** No tagged release ships the sign-in UI before Phase 4 is done. Sign-in alone does
nothing a user can see.

## Docs to update with the build

- `README.md`:
  - A "What HaryX sends, and where" section. Signed out: nothing new. Signed in: the HaryX sign-in
    email to Supabase, and everything else end-to-end encrypted.
  - Under "Use within Anthropic's terms": each machine signs in to Claude itself, and sync never
    copies a Claude login.
- The site: a privacy page naming Supabase as the processor.
- `architecture.md`:
  - The `server/` row becomes "Supabase project (`server/supabase/`); Go server later for team view".
  - New rules: "Claude logins never leave the device; only API keys and setup tokens sync, end-to-end
    encrypted. The apps never hold a HaryX token or the sync key."
- `roadmap.md`, `CHANGELOG.md`.

## Risks

- **Supabase Free can still pause.** "A few requests a day is usually enough", but that is not a
  guarantee. If it pauses, sync stops until the project is resumed from the dashboard; nothing local
  is lost. Free has no backups: the server is a relay, and `sync reset` rebuilds it from any machine.
  Upgrade to Pro (~$25/month) when users depend on it, or when storage nears 1 GB.
- **Google sign-in** needs a Google Cloud OAuth client with a verified consent screen, which can take
  days to weeks. The screen shows `<ref>.supabase.co` until there is a custom domain. GitHub has
  neither problem, so Google can land last without blocking.
- **Abuse.** Anyone can sign up. Rows are capped at 256 KB, and profiles at 3 × 10 MB per user.
  Supabase rate-limits sign-ups.
- **A wrong merge rewrites another machine's settings.** That is why overwrites are listed and
  profiles are never applied automatically.

## Later (not in this plan)

The email code and email + password methods (they need custom SMTP and a domain), team view, the Go
server, sign-in on the web dashboard and mobile, MFA, paid tiers, syncing folder mappings, and
real-time sync.

## Lessons

- **Security review, 2026-11-25 (Phase 1).** An independent review found that any 4xx from
  Supabase, including a 429 rate limit or a proxy's 403 page, signed the user out. Only GoTrue's own
  JSON 400/401/403 is a refusal now; everything else is a network error that keeps the session. The
  tests had stubbed only a clean 400. What would have caught it: a test per status class, not per
  happy path. The same review led to the random callback path, a per-connection deadline, a
  lock-held logout, and a session tied to the project that issued it.
- **Headless Chrome cannot render narrower than about 500 px.** Phone-width screenshots taken with
  `--window-size=390,…` are 500-px layouts, cropped. Render phone widths inside iframes of the
  real width instead.
