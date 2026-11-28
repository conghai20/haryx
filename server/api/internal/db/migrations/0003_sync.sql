-- Sync rows and profile files (plan task 6), with the rules the Supabase schema had: the same
-- kinds, keys, body sizes and caps. The server never sees plaintext: `body` is base64 age
-- ciphertext, and `key` and `name_hmac` are HMACs the client derives from its sync key.
--
-- The API scopes every query by user_id. Versions are its job: an update names the version it
-- read and raises it by one (internal/sync). The caps live here, as triggers that lock the
-- caller's users row first, so two requests at once cannot both take the last slot.

create table sync_rows (
    user_id uuid not null references users (user_id) on delete cascade,
    kind text not null constraint sync_rows_kind_check
        check (kind in ('keyring', 'settings', 'app-prefs', 'account', 'credential', 'profiles', 'usage')),
    key text not null default '' constraint sync_rows_key_check check (key ~ '^[0-9a-f]{0,64}$'),
    body text not null constraint sync_rows_body_check check (length(body) between 1 and 262144),
    version bigint not null default 1 constraint sync_rows_version_check check (version >= 1),
    updated_at timestamptz not null default now(),
    primary key (user_id, kind, key)
);

create table profile_files (
    user_id uuid not null references users (user_id) on delete cascade,
    name_hmac text not null constraint profile_files_name_check check (name_hmac ~ '^[0-9a-f]{1,64}$'),
    body bytea not null constraint profile_files_size_check check (octet_length(body) <= 10485760),
    updated_at timestamptz not null default now(),
    primary key (user_id, name_hmac)
);

-- 500 rows per user. A duplicate is let through, so it fails on the primary key (409), not here.
create function sync_rows_limit() returns trigger language plpgsql as $$
begin
    perform 1 from users where user_id = new.user_id for no key update;
    if (select count(*) from sync_rows where user_id = new.user_id) >= 500
       and not exists (select 1 from sync_rows
                       where user_id = new.user_id and kind = new.kind and key = new.key) then
        raise exception 'a user holds at most 500 sync rows'
            using errcode = 'check_violation', constraint = 'sync_rows_limit';
    end if;
    return new;
end
$$;

create trigger sync_rows_limit before insert on sync_rows
    for each row execute function sync_rows_limit();

-- 10 files per user. Replacing one the user already has is not a new file.
create function profile_files_limit() returns trigger language plpgsql as $$
begin
    perform 1 from users where user_id = new.user_id for no key update;
    if (select count(*) from profile_files where user_id = new.user_id) >= 10
       and not exists (select 1 from profile_files
                       where user_id = new.user_id and name_hmac = new.name_hmac) then
        raise exception 'a user holds at most 10 profile files'
            using errcode = 'check_violation', constraint = 'profile_files_limit';
    end if;
    return new;
end
$$;

create trigger profile_files_limit before insert on profile_files
    for each row execute function profile_files_limit();
