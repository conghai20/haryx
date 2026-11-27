<script lang="ts">
  // "Continue on…" for one running session: a menu of accounts, best first, each with its 5-hour %.
  // Picking one asks first, since it ends the running session (as macOS's menu does).
  import { ask } from "@tauri-apps/plugin-dialog";
  import { actions } from "../lib/actions";
  import { bestTarget, peakPct } from "../lib/handoff";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";
  import type { Account, Session } from "../lib/types";
  import MenuButton, { type MenuSection } from "./MenuButton.svelte";

  /** `from`: the account the session runs on, left out of the list. */
  let { session, from }: { session: Session; from: number | null } = $props();

  const fiveHour = (a: Account) => {
    const pct = a.usage?.fiveHour?.pct;
    return pct == null ? undefined : `${Math.round(pct)}%`;
  };

  async function confirmContinue(target: Account | null) {
    const name = target ? displayName(target) : "the best account";
    const folder = session.cwd || "its folder";
    const ok = await ask(
      `Ends the session in ${folder} and resumes the same conversation on ${name} in a new terminal.`,
      { title: `Continue on ${name}?`, kind: "warning", okLabel: "Continue", cancelLabel: "Cancel" },
    );
    if (ok) await actions.continueSession(session.pid, target?.number ?? null, session.cwd);
  }

  let sections = $derived.by<MenuSection[]>(() => {
    const best = bestTarget(store.accounts, from);
    const rest = store.accounts.filter((a) => a.number !== from && !a.disabled && a.number !== best?.number);
    const out: MenuSection[] = [];
    if (best) {
      out.push({
        heading: "Best account",
        items: [{ label: displayName(best), detail: fiveHour(best), onpick: () => confirmContinue(best) }],
      });
    }
    if (rest.length) {
      out.push({
        heading: best ? "Other accounts" : null,
        items: rest.map((a) => ({
          label: displayName(a),
          detail: fiveHour(a) ?? (peakPct(a) == null ? "no reading" : undefined),
          onpick: () => confirmContinue(a),
        })),
      });
    }
    return out;
  });
</script>

<MenuButton
  {sections}
  label="Continue on…"
  ariaLabel="Continue this session on another account"
  title="Ends this session and resumes the same conversation on another account, in a new terminal"
  disabled={sections.length === 0}
/>
