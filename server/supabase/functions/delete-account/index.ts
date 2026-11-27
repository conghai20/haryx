// Delete the calling HaryX user and everything they synced (`haryx account delete`).
//
// The caller proves who they are with their own access token; the function holds the only key
// allowed to delete users (SUPABASE_SERVICE_ROLE_KEY, provided by the platform, never in the repo).
// Profile files are removed first: storage objects do not cascade. Synced rows go with the user
// (`on delete cascade`).

import { createClient } from "npm:@supabase/supabase-js@2.99.3";

const BUCKET = "profiles";

function reply(status: number, body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}

Deno.serve(async (req) => {
  if (req.method !== "POST") {
    return reply(405, { error: "method_not_allowed" });
  }
  const token = (req.headers.get("authorization") ?? "").replace(/^Bearer\s+/i, "");
  if (!token) {
    return reply(401, { error: "not_signed_in" });
  }
  const admin = createClient(
    Deno.env.get("SUPABASE_URL")!,
    Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!,
    { auth: { persistSession: false, autoRefreshToken: false } },
  );
  const { data, error } = await admin.auth.getUser(token);
  if (error || !data.user) {
    return reply(401, { error: "not_signed_in" });
  }
  const userId = data.user.id;

  const files = await admin.storage.from(BUCKET).list(userId, { limit: 100 });
  if (files.error) {
    console.error("listing profile files failed", userId, files.error.message);
    return reply(500, { error: "storage_failed" });
  }
  if (files.data.length > 0) {
    const removed = await admin.storage
      .from(BUCKET)
      .remove(files.data.map((f) => `${userId}/${f.name}`));
    if (removed.error) {
      console.error("removing profile files failed", userId, removed.error.message);
      return reply(500, { error: "storage_failed" });
    }
  }

  const deleted = await admin.auth.admin.deleteUser(userId);
  if (deleted.error) {
    console.error("deleting the user failed", userId, deleted.error.message);
    return reply(500, { error: "delete_failed" });
  }
  return reply(200, { ok: true });
});
