-- HaryX encrypted sync (docs/haryx-account-plan.md, Phase 2).
--
-- One table of end-to-end encrypted rows per user. The server never sees plaintext: `body` is base64
-- age ciphertext, and `key` is an HMAC the client derives from its sync key, so emails and setting
-- names never reach the database. Each user reads and writes only their own rows (row-level
-- security). Writes are optimistic: an update must raise `version` by exactly one, so a client that
-- read a stale version updates nothing and pulls again.

create table public.sync_rows (
  user_id    uuid        not null default auth.uid() references auth.users (id) on delete cascade,
  kind       text        not null check (kind in ('keyring', 'settings', 'app-prefs', 'account', 'credential', 'profiles')),
  key        text        not null default '' check (key ~ '^[0-9a-f]{0,64}$'),
  body       text        not null check (length(body) between 1 and 262144),
  version    bigint      not null default 1 check (version >= 1),
  updated_at timestamptz not null default now(),
  primary key (user_id, kind, key)
);

comment on table public.sync_rows is
  'HaryX sync: end-to-end encrypted rows. body = base64 age ciphertext; key = client HMAC or empty.';

alter table public.sync_rows enable row level security;

create policy "own rows" on public.sync_rows
  for all to authenticated
  using ((select auth.uid()) = user_id)
  with check ((select auth.uid()) = user_id);

revoke all on public.sync_rows from anon;
grant select, insert, update, delete on public.sync_rows to authenticated;

-- The server owns updated_at, holds identity fixed, and enforces the one-step version bump.
create function public.sync_rows_guard() returns trigger
  language plpgsql
  set search_path = ''
as $$
begin
  if tg_op = 'INSERT' then
    new.version := 1;
    if (select count(*) from public.sync_rows where user_id = new.user_id) >= 500 then
      raise exception 'sync row limit reached' using errcode = 'check_violation';
    end if;
  else
    if new.user_id <> old.user_id or new.kind <> old.kind or new.key <> old.key then
      raise exception 'a sync row keeps its identity' using errcode = 'check_violation';
    end if;
    if new.version <> old.version + 1 then
      raise exception 'version must be % to update this row', old.version + 1
        using errcode = 'check_violation';
    end if;
  end if;
  new.updated_at := now();
  return new;
end
$$;

create trigger sync_rows_guard
  before insert or update on public.sync_rows
  for each row execute function public.sync_rows_guard();

-- Called once a day by .github/workflows/keepalive.yml so a Free project is not paused for
-- inactivity. Reads nothing.
create function public.ping() returns timestamptz
  language sql
  stable
  set search_path = ''
as $$ select now() $$;

revoke all on function public.ping() from public;
grant execute on function public.ping() to anon, authenticated;
