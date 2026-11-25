<script lang="ts">
  // One account in detail: usage bars, identity, and every per-account setting the CLI has.
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";
  import { FIVE_HOUR_S, SEVEN_DAY_S, displayName, needsRelogin, pctLabel, statusNote } from "../lib/tide";
  import { SPECIES, baseHue, speciesTitle, type Mood } from "../lib/mascot";
  import type { Account } from "../lib/types";
  import Mascot from "../components/Mascot.svelte";
  import TideStaff from "../components/TideStaff.svelte";

  let { account: a, onclose }: { account: Account; onclose: () => void } = $props();

  let style = $derived(store.styleFor(a));
  /** The colour slider's value while it is dragged, before it is saved. */
  let draggedHue = $state<number | null>(null);
  let hue = $derived(draggedHue ?? style.hue ?? baseHue(style.species));

  const moodCaptions: Record<Mood, string> = {
    running: "Working: a Claude session is using this account.",
    idle: "Idle.",
    tired: "Tired: 90% or more of a quota used.",
    sleeping: "Sleeping: disabled from rotation.",
    error: "",
  };
  let moodCaption = $derived.by(() => {
    const mood = store.moodFor(a);
    return mood === "error" ? `Needs attention: ${statusNote(a) ?? "check login"}.` : moodCaptions[mood];
  });

  async function saveHue() {
    if (draggedHue == null) return;
    await store.setMascotStyle(a, { species: style.species, hue: draggedHue });
    draggedHue = null;
  }

  let alias = $state("");
  let threshold = $state("");
  let priority = $state("");
  $effect(() => {
    alias = a.alias ?? "";
    threshold = a.threshold != null ? String(a.threshold) : "";
    priority = "";
    draggedHue = null;
  });

  function expiry(iso: string | null | undefined): string {
    if (!iso) return "unknown";
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
  }

  async function saveAlias() {
    const value = alias.trim();
    await actions.setAlias(a, value.length > 0 ? value : null);
  }

  async function saveThreshold() {
    const value = threshold.trim();
    if (value === "") return actions.setThreshold(a, null);
    const pct = Number(value);
    if (!Number.isFinite(pct) || pct < 50 || pct > 99.9) return store.flash("Threshold must be 50–99.9", true);
    await actions.setThreshold(a, pct);
  }

  async function savePriority() {
    const value = priority.trim();
    if (value === "") return actions.setPriority(a, null);
    const tier = Number(value);
    if (!Number.isInteger(tier) || tier < 0 || tier > 1000) return store.flash("Priority must be a whole number 0–1000", true);
    await actions.setPriority(a, tier);
  }
</script>

<aside aria-label="Account details">
  <header>
    <div class="who">
      <strong class="truncate">{displayName(a)}</strong>
      <span class="caption tertiary truncate">{a.email}{a.organizationName ? ` · ${a.organizationName}` : ""}</span>
    </div>
    <button class="plain" aria-label="Close details" onclick={onclose}>✕</button>
  </header>

  <section class="mascot" aria-label="Mascot">
    <Mascot account={a} size={112} style={{ species: style.species, hue: draggedHue ?? style.hue }} />
    <div class="field">
      <label for="species">Mascot</label>
      <select
        id="species"
        value={style.species}
        onchange={(e) => store.setMascotStyle(a, { species: e.currentTarget.value, hue: style.hue })}
      >
        {#each SPECIES as id (id)}<option value={id}>{speciesTitle(id)}</option>{/each}
      </select>
      <span></span>
    </div>
    <div class="field">
      <label for="hue">Colour</label>
      <input
        id="hue"
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={hue}
        style:accent-color="hsl({hue * 360} 76% 58%)"
        oninput={(e) => (draggedHue = Number(e.currentTarget.value))}
        onchange={saveHue}
      />
      <button class="mini" onclick={() => store.setMascotStyle(a, null)}>Reset</button>
    </div>
    <p class="caption secondary">{moodCaption}</p>
  </section>

  <div class="hairline"></div>

  {#each [{ label: "5-hour", w: a.usage?.fiveHour, s: FIVE_HOUR_S }, { label: "Week", w: a.usage?.sevenDay, s: SEVEN_DAY_S }] as bar (bar.label)}
    <div class="bar">
      <div class="bar-label caption">
        <span class="secondary">{bar.label}</span>
        {#if bar.w?.countdown}<span class="tertiary">resets in {bar.w.countdown}</span>{/if}
      </div>
      <div class="bar-row">
        <TideStaff window={bar.w} windowS={bar.s} now={store.now} height={5} label="{bar.label} usage" />
        <span class="pct">{pctLabel(bar.w?.pct)}</span>
      </div>
    </div>
  {/each}

  {#if statusNote(a)}<p class="caption note">{statusNote(a)}</p>{/if}
  <p class="caption secondary">Slot {a.number} · login expires {expiry(a.loginExpiresAt)}</p>

  <div class="hairline"></div>

  <div class="buttons">
    {#if !a.active}<button class="primary" onclick={() => actions.use(a)}>Switch</button>{/if}
    <button class:prominent={needsRelogin(a)} onclick={() => actions.signIn(a)}>Sign In Again</button>
    <button onclick={() => actions.runAs(a)}>Claude as this account</button>
  </div>

  <form class="field" onsubmit={(e) => { e.preventDefault(); saveAlias(); }}>
    <label for="alias">Alias</label>
    <input id="alias" bind:value={alias} placeholder="none" maxlength="40" />
    <button type="submit">Save</button>
  </form>
  <form class="field" onsubmit={(e) => { e.preventDefault(); saveThreshold(); }}>
    <label for="threshold">Switch away at</label>
    <input id="threshold" bind:value={threshold} placeholder="global default" inputmode="decimal" />
    <button type="submit">Save</button>
  </form>
  <form class="field" onsubmit={(e) => { e.preventDefault(); savePriority(); }}>
    <label for="priority">Priority tier</label>
    <input id="priority" bind:value={priority} placeholder="0" inputmode="numeric" />
    <button type="submit">Save</button>
  </form>

  <div class="hairline"></div>

  <div class="buttons">
    <button onclick={() => actions.setDisabled(a, !a.disabled)}>{a.disabled ? "Return to rotation" : "Hold out of rotation"}</button>
    <button class="danger" onclick={() => actions.remove(a)}>Remove…</button>
  </div>
</aside>

<style>
  aside {
    padding: 14px;
    border-radius: var(--radius-card);
    background: var(--fill);
    align-self: start;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: 13px;
  }
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .mascot .field {
    width: 100%;
  }
  .mascot p {
    align-self: flex-start;
  }
  .bar-label {
    display: flex;
    justify-content: space-between;
    font-weight: 500;
  }
  .bar-row {
    display: grid;
    grid-template-columns: 1fr 40px;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .pct {
    text-align: right;
    font-weight: 500;
  }
  .note {
    color: var(--amber);
    margin: 0;
  }
  p {
    margin: 0;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .prominent {
    background: var(--amber);
    color: white;
  }
  .danger {
    color: var(--red);
  }
  .field {
    display: grid;
    grid-template-columns: 110px 1fr auto;
    align-items: center;
    gap: 8px;
  }
  label {
    color: var(--label-2);
  }
</style>
