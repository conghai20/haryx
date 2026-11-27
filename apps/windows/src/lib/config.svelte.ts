// haryx's settings (`haryx config list/set/unset --json`), shared by Auto-switch (its rules) and AI
// settings (everything else). Each change saves at once; the list re-reads after it.

import { api } from "./api";
import { describeError } from "./errors";
import type { ConfigEntry } from "./settings";
import { store } from "./store.svelte";

class Config {
  entries = $state<ConfigEntry[] | null>(null);
  path = $state<string | null>(null);
  /** Plain words for a failed read; the raw text is in `errorDetail`. */
  error = $state<string | null>(null);
  errorDetail = $state<string | null>(null);

  async load() {
    try {
      const r = await api.json<{ path: string; settings: ConfigEntry[] }>(["config", "list", "--json"]);
      this.entries = r.settings;
      this.path = r.path;
      this.error = null;
      this.errorDetail = null;
    } catch (e) {
      const d = describeError(e, "Couldn't read haryx's settings");
      this.error = d.text;
      this.errorDetail = d.detail;
    }
  }

  /** Save `value`; blank resets to the default. */
  async set(key: string, value: string) {
    const v = value.trim();
    const ok = v === "" ? await store.run("Resetting setting", ["config", "unset", key]) : await store.run("Saving setting", ["config", "set", key, v], { success: "Saved" });
    if (ok) await this.load();
  }

  async unset(key: string) {
    if (await store.run("Resetting setting", ["config", "unset", key], { success: "Reset to default" })) await this.load();
  }
}

export const config = new Config();
