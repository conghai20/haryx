-- HaryX's own users (plan A10). A row is created by the user's first authenticated request and
-- touched by later ones. HaryX is never told that a WeCoAp ID was deleted, so a user with no
-- request for the retention period is deleted, and every table that holds their data references
-- this one `on delete cascade`.
create table users (
    user_id uuid primary key,
    first_seen timestamptz not null default now(),
    last_seen timestamptz not null default now()
);
create index users_last_seen on users (last_seen);
