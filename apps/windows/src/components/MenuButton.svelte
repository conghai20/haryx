<script lang="ts" module>
  export interface MenuItem {
    label: string;
    /** A second, quieter line or trailing figure, such as an account's 5-hour %. */
    detail?: string;
    onpick: () => unknown;
    disabled?: boolean;
    danger?: boolean;
  }
  /** A labelled block of items; a null heading draws only a separator. */
  export interface MenuSection {
    heading: string | null;
    items: MenuItem[];
  }
</script>

<script lang="ts">
  // A button that opens a menu below it: Windows' command-bar flyout. Closes on Escape, a click
  // elsewhere, or a pick; arrow keys move between items; focus returns to the button.
  import type { Snippet } from "svelte";

  let {
    sections,
    label,
    title,
    ariaLabel,
    plain = false,
    align = "end",
    disabled = false,
    children,
  }: {
    sections: MenuSection[];
    label?: string;
    title?: string;
    ariaLabel?: string;
    plain?: boolean;
    align?: "start" | "end";
    disabled?: boolean;
    children?: Snippet;
  } = $props();

  let open = $state(false);
  let trigger = $state<HTMLButtonElement>();
  let menu = $state<HTMLDivElement>();
  const id = `menu-${Math.random().toString(36).slice(2, 9)}`;

  function items(): HTMLButtonElement[] {
    return menu ? [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")] : [];
  }

  function show() {
    open = true;
    queueMicrotask(() => items()[0]?.focus());
  }

  function close(refocus = true) {
    open = false;
    if (refocus) trigger?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      return close();
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const list = items();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    list[(i + step + list.length) % list.length]?.focus();
  }

  function pick(item: MenuItem) {
    close();
    void item.onpick();
  }
</script>

<svelte:window onkeydown={onkeydown} onresize={() => open && close(false)} />

<span class="anchor">
  <button
    bind:this={trigger}
    class:plain
    {title}
    {disabled}
    aria-label={ariaLabel}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? id : undefined}
    onclick={() => (open ? close() : show())}
  >
    {#if children}{@render children()}{:else}{label}{/if}
  </button>
  {#if open}
    <!-- The backdrop only catches the click that closes the menu. -->
    <div class="backdrop" role="presentation" onclick={() => close(false)}></div>
    <div bind:this={menu} {id} class="menu" class:start={align === "start"} role="menu" aria-label={ariaLabel ?? label}>
      {#each sections as section, s (s)}
        {#if s > 0}<div class="sep" role="separator"></div>{/if}
        {#if section.heading}<p class="caption tertiary head" aria-hidden="true">{section.heading}</p>{/if}
        {#each section.items as item, i (i)}
          <button role="menuitem" class:danger={item.danger} disabled={item.disabled} onclick={() => pick(item)}>
            <span class="truncate">{item.label}</span>
            {#if item.detail}<span class="detail caption">{item.detail}</span>{/if}
          </button>
        {/each}
      {/each}
    </div>
  {/if}
</span>

<style>
  .anchor {
    position: relative;
    display: inline-flex;
  }
  .anchor > button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 21;
    min-width: 220px;
    max-width: 360px;
    max-height: 320px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--surface);
    border-radius: var(--radius-banner);
    /* Flyouts float above the page: Fluent's layered elevation, the one shadow the app draws. */
    box-shadow:
      0 0 0 1px var(--separator),
      0 8px 16px rgb(0 0 0 / 0.14);
  }
  .menu.start {
    right: auto;
    left: 0;
  }
  .menu button {
    display: flex;
    align-items: baseline;
    gap: 12px;
    text-align: left;
    background: transparent;
    padding: 6px 8px;
    min-width: 0;
  }
  .menu button:hover:not(:disabled),
  .menu button:focus-visible {
    background: var(--fill);
  }
  .menu .danger {
    color: var(--red);
  }
  .detail {
    margin-left: auto;
    flex: none;
  }
  .head {
    margin: 6px 8px 2px;
  }
  .sep {
    height: 1px;
    background: var(--separator);
    margin: 4px 0;
  }
</style>
