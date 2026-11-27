-- HaryX setup profiles carried between machines (docs/haryx-account-plan.md, Phase 3).
--
-- A private bucket of end-to-end encrypted files at `<user id>/<HMAC of the profile name>`. Each
-- user reaches only their own folder, and writes only directly in it (no subfolders: account
-- deletion lists the folder one level deep). The CLI keeps at most 3 profiles; the policy caps a folder at
-- 10 files, so a broken or hostile client cannot fill the project.

insert into storage.buckets (id, name, public, file_size_limit)
values ('profiles', 'profiles', false, 10485760)
on conflict (id) do nothing;

create policy "profiles: read own folder" on storage.objects
  for select to authenticated
  using (bucket_id = 'profiles' and (storage.foldername(name))[1] = (select auth.uid())::text);

-- How many profile files the caller has. Counting inside a policy on storage.objects would recurse
-- into that policy, so it runs as the owner, and it only ever counts the caller's own folder.
create function public.my_profile_file_count() returns bigint
  language sql
  stable
  security definer
  set search_path = ''
as $$
  select count(*) from storage.objects
  where bucket_id = 'profiles' and (storage.foldername(name))[1] = (select auth.uid())::text
$$;

revoke all on function public.my_profile_file_count() from public, anon;
grant execute on function public.my_profile_file_count() to authenticated;

create policy "profiles: add to own folder" on storage.objects
  for insert to authenticated
  with check (
    bucket_id = 'profiles'
    and (storage.foldername(name))[1] = (select auth.uid())::text
    and array_length(storage.foldername(name), 1) = 1
    and (select public.my_profile_file_count()) < 10
  );

create policy "profiles: replace in own folder" on storage.objects
  for update to authenticated
  using (bucket_id = 'profiles' and (storage.foldername(name))[1] = (select auth.uid())::text)
  with check (
    bucket_id = 'profiles'
    and (storage.foldername(name))[1] = (select auth.uid())::text
    and array_length(storage.foldername(name), 1) = 1
  );

create policy "profiles: delete from own folder" on storage.objects
  for delete to authenticated
  using (bucket_id = 'profiles' and (storage.foldername(name))[1] = (select auth.uid())::text);
