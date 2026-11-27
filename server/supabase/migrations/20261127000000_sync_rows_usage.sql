-- HaryX usage share between machines (docs/usage-share-plan.md).
--
-- A new row kind, `usage`: one end-to-end encrypted row per Claude account holding the newest
-- usage reading, the shared poll plan, the failure state and the request ledger, so every machine
-- of a user spends one usage-endpoint budget per account instead of one each. Rows follow the same
-- rules as every other kind (own rows only, optimistic versions, 500 rows per user).

-- The kind check was created inline, so its name is Postgres's choice: drop whichever check names
-- the kinds, then add it back under a fixed name.
do $$
declare
  existing text;
begin
  for existing in
    select conname from pg_constraint
    where conrelid = 'public.sync_rows'::regclass
      and contype = 'c'
      and pg_get_constraintdef(oid) like '%keyring%'
  loop
    execute format('alter table public.sync_rows drop constraint %I', existing);
  end loop;
end
$$;

alter table public.sync_rows add constraint sync_rows_kind_check
  check (kind in ('keyring', 'settings', 'app-prefs', 'account', 'credential', 'profiles', 'usage'));
