import { config } from "./config";
import { auth } from "./auth.svelte";

/** Step 1 of a machine sign-out: end that session through Keycloak's account API, as the user. */
export async function endSession(sessionId: string): Promise<void> {
  const res = await fetch(`${config.url}/realms/${config.realm}/account/sessions/${encodeURIComponent(sessionId)}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${await auth.token()}`, Accept: "application/json" },
  });
  // 404: already gone, which is what we wanted.
  if (!res.ok && res.status !== 404) throw new Error(`account API ${res.status}`);
}
