<script lang="ts">
  // A command to run on a Mac or PC, as the homepage sets it: Geist Mono in a ruled field with a
  // cyan prompt, and Copy beside it.
  import { Button } from "$lib/components/ui/button/index.js";
  let { command }: { command: string } = $props();
  let copyState = $state<"idle" | "copied" | "selected">("idle");
  let code = $state<HTMLElement>();

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      copyState = "copied";
    } catch {
      if (code) getSelection()?.selectAllChildren(code);
      copyState = "selected";
    }
    setTimeout(() => (copyState = "idle"), 1800);
  }
</script>

<div class="flex max-w-xl items-stretch gap-2">
  <div class="border-rule-strong flex min-w-0 flex-1 items-center gap-2 overflow-x-auto rounded-[2px] border px-3 py-2.5 font-mono text-[13px]">
    <span class="text-live-text select-none" aria-hidden="true">$</span>
    <code bind:this={code} class="whitespace-nowrap">{command}</code>
  </div>
  <Button variant={copyState === "copied" ? "outline" : "default"} onclick={copy} aria-live="polite">{copyState === "copied" ? "Copied" : copyState === "selected" ? "Selected" : "Copy"}</Button>
</div>
