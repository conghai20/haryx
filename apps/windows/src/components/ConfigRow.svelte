<script lang="ts">
  // One haryx setting in plain words, with a control matched to its value: a switch for on/off, a
  // picker for a fixed set, a number or text field otherwise. Saves on change (a field on Enter or
  // when focus leaves). The dotted key is in the tooltip.
  import { config } from "../lib/config.svelte";
  import { settingText, type ConfigEntry } from "../lib/settings";
  import Icon from "./Icon.svelte";

  let { entry }: { entry: ConfigEntry } = $props();

  let text = $derived(settingText(entry.key));
  let id = $derived(`cfg-${entry.key.replace(/\W/g, "-")}`);
  let current = $derived(entry.value == null ? "" : String(entry.value));
  /** The on/off setting this one needs is off: the control is shown but can't be used. */
  let blocked = $derived(text.needs != null && config.entries?.find((e) => e.key === text.needs)?.value === false);

  function commit(e: Event & { currentTarget: HTMLInputElement }) {
    const v = e.currentTarget.value.trim();
    if (v === current) return;
    void config.set(entry.key, v);
  }
</script>

<div class="row" title={entry.key}>
  <div class="words">
    <label for={id}>{text.label}</label>
    {#if text.caption}<span class="caption">{text.caption}</span>{/if}
  </div>
  <div class="control">
    {#if text.choices}
      <select {id} value={current} onchange={(e) => config.set(entry.key, e.currentTarget.value)}>
        {#each text.choices as c (c)}<option value={c}>{text.choiceLabels?.[c] ?? c}</option>{/each}
      </select>
    {:else if typeof entry.value === "boolean"}
      <input
        {id}
        type="checkbox"
        role="switch"
        class="switch"
        disabled={blocked}
        checked={entry.value}
        onchange={(e) => config.set(entry.key, e.currentTarget.checked ? "true" : "false")}
      />
    {:else}
      <input
        {id}
        class="field"
        inputmode={text.unit ? "decimal" : "text"}
        value={current}
        placeholder="default"
        onchange={commit}
        onkeydown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") e.currentTarget.value = current;
        }}
      />
      <!-- A blank field uses the default: no unit after it. -->
      <span class="unit caption" aria-hidden={!text.unit || current === ""}>{current === "" ? "" : (text.unit ?? "")}</span>
    {/if}
    {#if entry.isSet}
      <button class="plain" aria-label="Reset {text.label} to default" title="Reset to default" onclick={() => config.unset(entry.key)}>
        <Icon name="undo" size={14} />
      </button>
    {:else}
      <span class="reset-space" aria-hidden="true"></span>
    {/if}
  </div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 0;
    border-top: 1px solid var(--separator);
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .control {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .field {
    width: 96px;
    text-align: right;
  }
  .unit {
    width: 24px;
  }
  .reset-space {
    width: 26px;
  }
  .plain {
    display: inline-flex;
  }
</style>
