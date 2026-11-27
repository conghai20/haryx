-- Row-level security and write rules of public.sync_rows. Run: supabase test db (from server/).
begin;
create extension if not exists pgtap with schema extensions;
select plan(17);

insert into auth.users (id, email) values
  ('00000000-0000-0000-0000-00000000000a', 'a@example.test'),
  ('00000000-0000-0000-0000-00000000000b', 'b@example.test');

-- Act as a signed-in user, as PostgREST does for a request with that user's JWT.
create function pg_temp.act_as(uid text) returns void language sql as $$
  select set_config('role', 'authenticated', true),
         set_config('request.jwt.claims', json_build_object('sub', uid, 'role', 'authenticated')::text, true);
$$;

-- User A writes.
select pg_temp.act_as('00000000-0000-0000-0000-00000000000a');
select lives_ok(
  $$ insert into public.sync_rows (kind, key, body) values ('settings', '', 'YWdlLWE=') $$,
  'a user stores a row');
select is((select user_id::text from public.sync_rows where kind = 'settings'),
  '00000000-0000-0000-0000-00000000000a', 'user_id defaults to the caller');
select is((select version from public.sync_rows where kind = 'settings'), 1::bigint,
  'a new row starts at version 1');
select throws_ok(
  $$ insert into public.sync_rows (user_id, kind, key, body)
     values ('00000000-0000-0000-0000-00000000000b', 'settings', '', 'eA==') $$,
  '42501', null, 'a user cannot write a row for someone else');
select throws_ok(
  $$ insert into public.sync_rows (kind, key, body) values ('secrets', '', 'eA==') $$,
  '23514', null, 'unknown kinds are refused');
select lives_ok(
  $$ insert into public.sync_rows (kind, key, body) values ('usage', 'ab12', 'eA==') $$,
  'usage rows are accepted');
select throws_ok(
  $$ insert into public.sync_rows (kind, key, body) values ('account', 'You@Example.com', 'eA==') $$,
  '23514', null, 'a key is a lowercase hex digest, never a readable name');
select lives_ok(
  $$ update public.sync_rows set body = 'Yg==', version = 2 where kind = 'settings' and version = 1 $$,
  'an update that raises the version by one is accepted');
select throws_ok(
  $$ update public.sync_rows set body = 'Yw==', version = 5 where kind = 'settings' $$,
  '23514', null, 'a version that skips ahead is refused');
select is((select body from public.sync_rows where kind = 'settings'), 'Yg==',
  'the accepted update is stored');

-- User B sees and changes nothing of A's.
select pg_temp.act_as('00000000-0000-0000-0000-00000000000b');
select is((select count(*) from public.sync_rows), 0::bigint, 'another user sees no rows');
update public.sync_rows set body = 'aGFjaw==', version = 3 where kind = 'settings';
delete from public.sync_rows where kind = 'settings';

select pg_temp.act_as('00000000-0000-0000-0000-00000000000a');
select is((select body from public.sync_rows where kind = 'settings'), 'Yg==',
  'another user can neither update nor delete it');

-- The anonymous key reaches the keep-alive and nothing else.
reset role;
set local role anon;
select throws_ok($$ select count(*) from public.sync_rows $$, '42501', null,
  'the anonymous role cannot read rows');
select isnt(public.ping(), null, 'the anonymous role can call ping()');

-- Deleting the user removes their rows.
reset role;
select is((select count(*) from public.sync_rows), 2::bigint, 'two rows before the account is deleted');
delete from auth.users where id = '00000000-0000-0000-0000-00000000000a';
select is((select count(*) from public.sync_rows), 0::bigint, 'deleting the user deletes their rows');

-- Per-user cap.
insert into public.sync_rows (user_id, kind, key, body)
  select '00000000-0000-0000-0000-00000000000b', 'account', lpad(to_hex(n), 64, '0'), 'eA=='
  from generate_series(1, 500) as n;
select throws_ok(
  $$ insert into public.sync_rows (user_id, kind, key, body)
     values ('00000000-0000-0000-0000-00000000000b', 'account', lpad(to_hex(501), 64, '0'), 'eA==') $$,
  '23514', null, 'a user holds at most 500 rows');

select * from finish();
rollback;
