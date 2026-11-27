-- Storage policies of the private `profiles` bucket. Run: supabase test db (from server/).
begin;
create extension if not exists pgtap with schema extensions;
select plan(6);

insert into auth.users (id, email) values
  ('00000000-0000-0000-0000-0000000000a1', 'a@example.test'),
  ('00000000-0000-0000-0000-0000000000b1', 'b@example.test');

create function pg_temp.act_as(uid text) returns void language sql as $$
  select set_config('role', 'authenticated', true),
         set_config('request.jwt.claims', json_build_object('sub', uid, 'role', 'authenticated')::text, true);
$$;

select is((select public from storage.buckets where id = 'profiles'), false, 'the bucket is private');

select pg_temp.act_as('00000000-0000-0000-0000-0000000000a1');
select lives_ok(
  $$ insert into storage.objects (bucket_id, name) values ('profiles', '00000000-0000-0000-0000-0000000000a1/f1') $$,
  'a user stores a file in their own folder');
select throws_ok(
  $$ insert into storage.objects (bucket_id, name) values ('profiles', '00000000-0000-0000-0000-0000000000b1/f1') $$,
  '42501', null, 'a user cannot write into someone else''s folder');
select throws_ok(
  $$ insert into storage.objects (bucket_id, name) values ('profiles', '00000000-0000-0000-0000-0000000000a1/sub/f2') $$,
  '42501', null, 'no subfolders: deletion lists one level');

select pg_temp.act_as('00000000-0000-0000-0000-0000000000b1');
select is((select count(*) from storage.objects where bucket_id = 'profiles'), 0::bigint,
  'another user sees no files');
-- (Deletes are refused for everyone in SQL by Storage's own trigger; the API path is covered by
-- tests/cli-e2e.sh.)
select pg_temp.act_as('00000000-0000-0000-0000-0000000000a1');

-- The folder cap.
insert into storage.objects (bucket_id, name)
  select 'profiles', '00000000-0000-0000-0000-0000000000a1/g' || n from generate_series(2, 10) as n;
select throws_ok(
  $$ insert into storage.objects (bucket_id, name) values ('profiles', '00000000-0000-0000-0000-0000000000a1/g11') $$,
  '42501', null, 'a folder holds at most 10 files');

select * from finish();
rollback;
