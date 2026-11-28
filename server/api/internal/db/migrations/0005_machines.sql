-- The machine registry (plan task 8): one row per machine that heartbeats with the `machines`
-- consent. Plaintext by design, so the web can list, rename and sign out machines without the sync
-- passphrase. `session_id` is the `sid` of the machine's latest token: signing the machine out
-- revokes it (revoked_sessions) and drops the row. It is never sent to a client.

create table machines (
    user_id uuid not null references users (user_id) on delete cascade,
    machine_id uuid not null,
    name text not null constraint machines_name_check
        check (char_length(name) between 1 and 64 and name !~ '[[:cntrl:]]'),
    os text not null constraint machines_os_check check (os in ('macos', 'windows', 'linux')),
    version text not null constraint machines_version_check
        check (version ~ '^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$'),
    session_id text not null constraint machines_session_check check (session_id <> ''),
    last_seen_at timestamptz not null,
    primary key (user_id, machine_id)
);

-- No machine row without the machines consent (see require_consent in 0004_consents.sql). A
-- heartbeat is an insert … on conflict, and before-insert triggers fire for it too.
create trigger machines_consent before insert on machines
    for each row execute function require_consent('machines');
