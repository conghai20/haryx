-- The HaryX consents (plan task 7, Law 91): `sync` and `machines`, each granted and withdrawn on
-- its own. `consents` is the current state, one row per answered purpose; `consent_events` is the
-- proof, one row per grant or withdrawal, never changed. Events outlive the data a withdrawal
-- deletes, and go only with the user (A10).

create table consents (
    user_id uuid not null references users (user_id) on delete cascade,
    purpose text not null constraint consents_purpose_check check (purpose in ('sync', 'machines')),
    granted boolean not null,
    version text not null, -- of the latest event
    at timestamptz not null, -- of the latest event
    primary key (user_id, purpose)
);

create table consent_events (
    id bigint generated always as identity primary key,
    user_id uuid not null references users (user_id) on delete cascade,
    purpose text not null constraint consent_events_purpose_check check (purpose in ('sync', 'machines')),
    action text not null constraint consent_events_action_check check (action in ('grant', 'withdraw')),
    version text not null constraint consent_events_version_check check (version ~ '^[0-9A-Za-z][0-9A-Za-z._-]{0,31}$'),
    at timestamptz not null default now(),
    source text not null constraint consent_events_source_check check (source in ('cli', 'macos', 'windows', 'web', 'sso'))
);
create index consent_events_user on consent_events (user_id, purpose, id);

-- Append-only: an event is proof, so it is never rewritten. Deleting is left to the users cascade.
create function consent_events_frozen() returns trigger language plpgsql as $$
begin
    raise exception 'consent_events are append-only';
end
$$;

create trigger consent_events_frozen before update on consent_events
    for each statement execute function consent_events_frozen();

-- Data of a purpose can only be written while its consent is granted. The API's gate already
-- answers 403 first; this closes the race with a withdrawal running at the same time. Both lock
-- the caller's users row, so a withdrawal waits for an insert in flight, or the insert sees it.
-- Task 8: `machines` gets the same trigger with 'machines'.
create function require_consent() returns trigger language plpgsql as $$
begin
    perform 1 from users where user_id = new.user_id for no key update;
    if not exists (select 1 from consents
                   where user_id = new.user_id and purpose = tg_argv[0] and granted) then
        raise exception 'no % consent', tg_argv[0]
            using errcode = 'check_violation', constraint = 'consent_required';
    end if;
    return new;
end
$$;

create trigger sync_rows_consent before insert on sync_rows
    for each row execute function require_consent('sync');
create trigger profile_files_consent before insert on profile_files
    for each row execute function require_consent('sync');
