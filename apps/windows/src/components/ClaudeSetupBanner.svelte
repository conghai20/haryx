<script lang="ts">
  // Shown while Claude Code is missing or has never signed in (`haryx ai doctor`): nothing in the AI
  // section works until it is fixed, so say what is wrong and offer the fix in one click. As macOS.
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";

  /** The flyout shows the problem and its main button only. */
  let { compact = false }: { compact?: boolean } = $props();
  let checking = $state(false);

  let problem = $derived(store.claudeSetup && !store.claudeSetup.ready ? store.claudeSetup.problems[0] : null);

  async function checkAgain() {
    checking = true;
    await store.checkClaudeSetup();
    checking = false;
  }
</script>

{#if problem}
  <div class="setup" class:compact role="status">
    <strong>⚠ {problem.message}</strong>
    {#if !compact}
      <!-- The fix carries commands in backticks: odd segments are code. -->
      <p class="caption">
        {#each problem.fix.split("`") as part, i}{#if i % 2}<code>{part}</code>{:else}{part}{/if}{/each}
      </p>
    {/if}
    <div class="buttons">
      {#if problem.code === "claude_missing"}
        <button class="primary" onclick={() => actions.terminal({ kind: "install-claude" })}>Install Claude Code</button>
      {:else if problem.code === "not_signed_in"}
        <button class="primary" onclick={() => actions.terminal({ kind: "claude" })}>Open Claude to Sign In</button>
      {/if}
      <button disabled={checking} onclick={checkAgain}>Check Again</button>
    </div>
  </div>
{/if}

<style>
  .setup {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px;
    border-radius: var(--radius-banner);
    background: color-mix(in srgb, var(--amber) 10%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--amber) 35%, transparent);
  }
  .setup.compact {
    margin-top: 8px;
    padding: 10px;
    font-size: 12px;
  }
  strong {
    color: var(--amber);
  }
  p {
    margin: 0;
    color: var(--label-2);
    user-select: text;
  }
  .buttons {
    display: flex;
    gap: 8px;
  }
</style>
