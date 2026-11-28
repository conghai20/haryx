-- Sessions signed out from the web (plan task 8). Keycloak already refuses their refresh token;
-- this refuses their access tokens for the minutes they have left. `until` is when the last of
-- them has expired, so the row can go.
create table revoked_sessions (
    session_id text primary key,
    until timestamptz not null
);
