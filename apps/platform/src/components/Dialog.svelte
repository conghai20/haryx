<script lang="ts">
  // Native <dialog>: showModal() traps focus, Esc closes, backdrop blocks the page.
  import type { Snippet } from "svelte";
  let { title, onClose, children }: { title: string; onClose: () => void; children: Snippet } = $props();
  let el = $state<HTMLDialogElement>();
  $effect(() => { el?.showModal(); });
</script>

<dialog bind:this={el} onclose={onClose} aria-labelledby="dlg-title">
  <h2 id="dlg-title">{title}</h2>
  {@render children()}
</dialog>

<style>
  dialog { background: var(--surface); color: var(--label); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: 1.25rem; width: min(26rem, calc(100vw - 2rem)); }
  dialog::backdrop { background: rgb(0 0 0 / .4); }
</style>
