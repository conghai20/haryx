# HaryX on Supabase

The backend of HaryX sign-in and sync (plan: [`docs/haryx-account-plan.md`](../../docs/haryx-account-plan.md)).
Hosted project: `ygvcznwlugaxqqmlsssh` (Free plan, kept awake by `.github/workflows/keepalive.yml`).

| Path | What |
|---|---|
| `migrations/20261125000000_sync_rows.sql` | `public.sync_rows` (end-to-end encrypted rows, row-level security, one-step versions, 500 rows per user) and `public.ping()` |
| `migrations/20261125000100_profiles_bucket.sql` | the private `profiles` bucket (10 MB files, 10 per user) and its policies |
| `functions/delete-account/` | `haryx account delete`: removes the caller's files, then the user (rows cascade) |
| `tests/*.sql` | pgTAP: isolation between users, version rules, anon access, caps |
| `tests/cli-e2e.sh` | the real `haryx` binary as two machines against the local stack |
| `config.toml` | the local stack; GitHub sign-in reads `SUPABASE_AUTH_GITHUB_CLIENT_ID` / `_SECRET` |

## Local

```sh
cd server
npx supabase@3.1.0 start        # Docker; applies the migrations, serves the function
npx supabase@3.1.0 test db      # pgTAP
(cd ../cli && cargo build) && sh supabase/tests/cli-e2e.sh
```

## Deploying to the hosted project

Needs a Supabase access token for the project owner: `npx supabase@3.1.0 login` in a real terminal
(it cannot run without one). Run from an agent, hosted changes also need `--yes`.

```sh
cd server
npx supabase@3.1.0 link --project-ref ygvcznwlugaxqqmlsssh
npx supabase@3.1.0 db push --remote                     # both migrations (without --remote: the local stack)
npx supabase@3.1.0 functions deploy delete-account --remote=ygvcznwlugaxqqmlsssh
```

Dashboard settings that are not in this repo: GitHub (and later Google) provider credentials under
Authentication › Sign In / Providers; Site URL `https://haryx.wecoap.com/`. Loopback
redirects need no allow-list entry. Email sign-up can stay off until custom SMTP exists.
