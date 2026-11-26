<script lang="ts">
  // One account in detail, in the same order as macOS's inspector: who it is and its state, usage,
  // live sessions, switching settings, then rarely used housekeeping under Advanced. Every setting
  // saves as soon as it changes; text fields save on Enter or when focus leaves (the `change` event).
  import { untrack } from "svelte";
  import { actions } from "../lib/actions";
  import { GROUP_MAX_CHARS, accountGroups, groupNames, isInGroup } from "../lib/groups";
  import { bestTarget, peakPct } from "../lib/handoff";
  import { store } from "../lib/store.svelte";
  import {
    FIVE_HOUR_S,
    SEVEN_DAY_S,
    displayName,
    needsRelogin,
    offersSignIn,
    pctLabel,
    statusNote,
    usageFetchedLabel,
    weeklyPace,
  } from "../lib/tide";
  import { SPECIES, baseHue, speciesTitle, type Mood } from "../lib/mascot";
  import type { Account, Session } from "../lib/types";
  import Mascot from "../components/Mascot.svelte";
  import TideStaff from "../components/TideStaff.svelte";

  let { account: a, onclose }: { account: Account; onclose: () => void } = $props();

  /** The slot range macOS's Move stepper allows. */
  const SLOT_MIN = 1;
  const SLOT_MAX = 99;
  /** The CLI's threshold range (macOS's slider). */
  const THRESHOLD_MIN = 50;
  const THRESHOLD_MAX = 99.9;
  /** The slider's starting point when no threshold is set. */
  const THRESHOLD_DEFAULT = 90;

  let style = $derived(store.styleFor(a));
  /** The colour slider's value while it is dragged, before it is saved. */
  let draggedHue = $state<number | null>(null);
  let hue = $derived(draggedHue ?? style.hue ?? baseHue(style.species));

  const moodCaptions: Record<Mood, string> = {
    running: "Working: a Claude session is using this account.",
    idle: "Idle.",
    angry: "Angry: 75% or more of a quota used.",
    tired: "Tired: 90% or more of a quota used.",
    sleeping: "Sleeping: out of rotation.",
    error: "",
  };
  let moodCaption = $derived.by(() => {
    const mood = store.moodFor(a);
    if (mood === "error") return `Needs attention: ${statusNote(a) ?? "check login"}.`;
    return mood === "sleeping" && !a.disabled ? "Sleeping: at its limit until the quota resets." : moodCaptions[mood];
  });

  let sessions = $derived(
    store.sessions.sessions.filter((s) => s.account === a.number).sort((x, y) => y.startedAt - x.startedAt),
  );

  let stateText = $derived.by(() => {
    const n = sessions.length;
    const running = n === 0 ? null : `${n} session${n === 1 ? "" : "s"} running`;
    if (a.active) return ["In use", running].filter(Boolean).join(" · ");
    if (running) return running;
    if (needsRelogin(a)) return "Sign-in needed";
    if (a.disabled) return "Out of rotation";
    return "Ready";
  });
  let stateClass = $derived(a.active || sessions.length > 0 ? "live" : needsRelogin(a) ? "warn" : "");

  /** The email and organisation when the title is an alias; else just the organisation. */
  let detail = $derived(
    [a.alias ? a.email : null, a.organizationName].filter((x): x is string => !!x).join(" · "),
  );

  // The fields follow the account's saved values, but only when those values change, so a refresh
  // of the account list never overwrites what is being typed.
  let alias = $state("");
  let group = $state("");
  let thresholdOn = $state(false);
  let threshold = $state(THRESHOLD_DEFAULT);
  let priority = $state("");
  let moveSlot = $state(1);
  let swapSlot = $state("");
  let showAdvanced = $state(false);

  let savedAlias = $derived(a.alias ?? "");
  let savedThreshold = $derived(a.threshold ?? null);
  let slot = $derived(a.number);

  $effect(() => {
    alias = savedAlias;
  });
  $effect(() => {
    thresholdOn = savedThreshold != null;
    if (savedThreshold != null) threshold = savedThreshold;
  });
  $effect(() => {
    moveSlot = slot;
    untrack(() => {
      priority = "";
      swapSlot = "";
      draggedHue = null;
    });
  });

  async function saveHue() {
    if (draggedHue == null) return;
    await store.setMascotStyle(a, { species: style.species, hue: draggedHue });
    draggedHue = null;
  }

  async function saveAlias() {
    const value = alias.trim();
    if (value === savedAlias) return;
    await actions.setAlias(a, value.length > 0 ? value : null);
  }

  /** The group field adds one group (an account can be in several), then empties. */
  async function addGroup() {
    const value = group.trim();
    group = "";
    if (value.length === 0 || isInGroup(a, value)) return;
    await actions.addToGroup(a, value);
  }

  async function setThresholdOn(on: boolean) {
    thresholdOn = on;
    await actions.setThreshold(a, on ? threshold : null);
  }

  async function savePriority() {
    const value = priority.trim();
    if (value === "" || value === "0") return actions.setPriority(a, null);
    const tier = Number(value);
    if (!Number.isInteger(tier) || tier < 0 || tier > 1000) return store.flash("Priority must be a whole number 0–1000", true);
    await actions.setPriority(a, tier);
  }

  async function moveToSlot() {
    const target = Number(moveSlot);
    if (!Number.isInteger(target) || target < SLOT_MIN || target > SLOT_MAX)
      return store.flash(`Position must be a whole number ${SLOT_MIN}–${SLOT_MAX}`, true);
    // Keep this account selected at its new slot.
    if (await actions.move(a, target)) store.revealSlot = target;
  }

  async function swapWith() {
    const other = store.accounts.find((x) => String(x.number) === swapSlot);
    if (other && (await actions.swap(a, other))) store.revealSlot = other.number;
  }

  function continueOn(s: Session, value: string) {
    if (!value) return;
    actions.continueSession(s.pid, value === "best" ? null : Number(value), s.cwd);
  }

  function since(ms: number): string {
    const minutes = Math.max(0, Math.round((store.now - ms) / 60_000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    return hours < 24 ? `${hours}h ${minutes % 60}m` : `${Math.floor(hours / 24)}d ${hours % 24}h`;
  }

  function when(iso: string | null | undefined): string | null {
    if (!iso) return null;
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
  }
</script>

<aside aria-label="Account details">
  <header>
    <span title={moodCaption}>
      <Mascot account={a} size={56} style={{ species: style.species, hue: draggedHue ?? style.hue }} />
    </span>
    <div class="who">
      <strong class="name truncate">{displayName(a)}</strong>
      {#if detail}<span class="secondary truncate">{detail}</span>{/if}
      <span class="state {stateClass}">{stateText}</span>
    </div>
    <button class="plain" aria-label="Close details" onclick={onclose}>✕</button>
  </header>

  {#if statusNote(a)}<p class="note">{statusNote(a)}</p>{/if}

  <div class="buttons">
    {#if !a.active}
      <button class="primary" disabled={!!store.busy} onclick={() => actions.use(a)}>Switch</button>
    {/if}
    {#if offersSignIn(a, store.now)}
      <button class:prominent={needsRelogin(a)} onclick={() => actions.signIn(a)}>Sign In Again</button>
    {/if}
    <button onclick={() => actions.runAs(a)}>Run Claude</button>
    <button onclick={() => actions.runInFolder(a)}>Run in Folder…</button>
  </div>

  <section aria-labelledby="insp-usage">
    <h3 id="insp-usage">Usage</h3>
    {#if a.usageError}<p class="note">{a.usageError}</p>{/if}
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
    {#if weeklyPace(a.usage?.sevenDay, store.now)}
      {@const pace = weeklyPace(a.usage?.sevenDay, store.now)}
      <p class="row"><span>Weekly pace</span><span class:note={pace?.warn} class:secondary={!pace?.warn}>{pace?.text}</span></p>
    {/if}
    {#if a.usage?.sevenDay?.willLastToReset === false && when(a.usage.sevenDay.projectedExhaustionAt)}
      <p class="row"><span>Projected run-out</span><span class="secondary">{when(a.usage.sevenDay.projectedExhaustionAt)}</span></p>
    {/if}
    {#if when(a.loginExpiresAt)}<p class="caption secondary">Login expires {when(a.loginExpiresAt)}</p>{/if}
    {#if usageFetchedLabel(a)}<p class="caption secondary">{usageFetchedLabel(a)}</p>{/if}
  </section>

  {#if sessions.length > 0}
    <section aria-labelledby="insp-sessions">
      <h3 id="insp-sessions">Live Claude sessions</h3>
      {#each sessions as s (s.pid)}
        <div class="session">
          <div class="session-text">
            <span class="truncate" title={s.cwd}>{s.cwd || `pid ${s.pid}`}</span>
            <span class="caption tertiary">pid {s.pid} · running for {since(s.startedAt)}{s.profile === "session" ? " · session mode" : ""}</span>
          </div>
          <select
            aria-label="Continue this session on another account"
            title="Ends this session and resumes the same conversation on another account, in a new terminal"
            value=""
            onchange={(e) => {
              continueOn(s, e.currentTarget.value);
              e.currentTarget.value = "";
            }}
          >
            <option value="">Continue on…</option>
            {#if bestTarget(store.accounts, a.number)}<option value="best">Best account</option>{/if}
            {#each store.accounts.filter((x) => x.number !== a.number && !x.disabled) as x (x.number)}
              <option value={String(x.number)}>{x.number}. {displayName(x)}{peakPct(x) != null ? ` (${peakPct(x)}%)` : ""}</option>
            {/each}
          </select>
        </div>
      {/each}
    </section>
  {/if}

  <section aria-labelledby="insp-switching">
    <h3 id="insp-switching">Switching</h3>
    <label class="line">
      <input type="checkbox" checked={!a.disabled} onchange={(e) => actions.setDisabled(a, !e.currentTarget.checked)} />
      Included in rotation
    </label>
    <div class="field">
      <label for="group">Groups</label>
      <input
        id="group"
        bind:value={group}
        placeholder={accountGroups(a).length > 0 ? "add another" : "none"}
        maxlength={GROUP_MAX_CHARS}
        list="group-names"
        onchange={addGroup}
        onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      <datalist id="group-names">
        {#each groupNames(store.accounts).filter((n) => !isInGroup(a, n)) as name (name)}<option value={name}></option>{/each}
      </datalist>
    </div>
    {#if accountGroups(a).length > 0}
      <ul class="group-chips" aria-label="{displayName(a)}'s groups">
        {#each accountGroups(a) as name (name)}
          <li>
            {name}
            <button
              class="plain"
              aria-label="Remove from {name}"
              title="Take {displayName(a)} out of {name}"
              disabled={!!store.busy}
              onclick={() => actions.removeFromGroup(a, name)}>✕</button
            >
          </li>
        {/each}
      </ul>
    {/if}
    <label class="line">
      <input type="checkbox" checked={thresholdOn} onchange={(e) => setThresholdOn(e.currentTarget.checked)} />
      Custom switch-away threshold
    </label>
    {#if thresholdOn}
      <div class="field">
        <label for="threshold">Switch away at</label>
        <div class="range">
          <input
            id="threshold"
            type="range"
            min={THRESHOLD_MIN}
            max={THRESHOLD_MAX}
            step="0.5"
            bind:value={threshold}
            aria-valuetext="{threshold.toFixed(1)}%"
            onchange={() => actions.setThreshold(a, threshold)}
          />
          <span class="pct">{threshold.toFixed(1)}%</span>
        </div>
      </div>
    {/if}
    <p class="caption secondary">
      Automatic switching tries accounts that share a group with the active one first, and moves off this account
      once its usage passes the threshold.
    </p>
  </section>

  <details bind:open={showAdvanced}>
    <summary>Advanced</summary>
    <div class="advanced">
      <div class="field">
        <label for="species">Mascot</label>
        <select
          id="species"
          value={style.species}
          onchange={(e) => store.setMascotStyle(a, { species: e.currentTarget.value, hue: style.hue })}
        >
          {#each SPECIES as id (id)}<option value={id}>{speciesTitle(id)}</option>{/each}
        </select>
      </div>
      <div class="field">
        <label for="hue">Mascot colour</label>
        <div class="range">
          <input
            id="hue"
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={hue}
            aria-valuetext={style.hue == null && draggedHue == null ? "default" : `hue ${Math.round(hue * 360)} degrees`}
            style:accent-color="hsl({hue * 360} 76% 58%)"
            oninput={(e) => (draggedHue = Number(e.currentTarget.value))}
            onchange={saveHue}
          />
          <button class="mini" onclick={() => store.setMascotStyle(a, null)}>Reset</button>
        </div>
      </div>
      <div class="field">
        <label for="alias">Alias</label>
        <input
          id="alias"
          bind:value={alias}
          placeholder="none"
          title="A short name shown instead of the email: letters, digits, . - _"
          maxlength="40"
          onchange={saveAlias}
          onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
      </div>
      <div class="field">
        <label for="priority">Priority</label>
        <input
          id="priority"
          bind:value={priority}
          placeholder="none"
          inputmode="numeric"
          title="Read only by the priority-consume-first auto-switch strategy"
          onchange={savePriority}
          onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
      </div>
      <form class="field" onsubmit={(e) => { e.preventDefault(); moveToSlot(); }}>
        <label for="move">Position in list</label>
        <div class="range">
          <input id="move" type="number" min={SLOT_MIN} max={SLOT_MAX} step="1" bind:value={moveSlot} />
          <button type="submit" disabled={moveSlot === a.number || !!store.busy}>Move</button>
        </div>
      </form>
      <form class="field" onsubmit={(e) => { e.preventDefault(); swapWith(); }}>
        <label for="swap">Swap places with</label>
        <div class="range">
          <select id="swap" bind:value={swapSlot}>
            <option value="">—</option>
            {#each store.accounts.filter((x) => x.number !== a.number) as x (x.number)}
              <option value={String(x.number)}>{x.number}: {displayName(x)}</option>
            {/each}
          </select>
          <button type="submit" disabled={swapSlot === "" || !!store.busy}>Swap</button>
        </div>
      </form>
      <div>
        <button
          disabled={a.active}
          title="Activate the stored credentials without backing up the current login (--force)"
          onclick={() => actions.forceUse(a)}>Switch Without Saving Current Login…</button
        >
      </div>
    </div>
  </details>

  <div class="hairline"></div>
  <div>
    <button class="danger" onclick={() => actions.remove(a)}>Remove Account…</button>
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
    gap: 12px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-size: 16px;
    font-weight: 600;
  }
  .state {
    font-weight: 500;
    color: var(--label-2);
  }
  .state.live {
    color: var(--cyan-text);
  }
  .state.warn {
    color: var(--amber);
  }
  header .plain {
    align-self: flex-start;
  }
  section,
  .advanced {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  section {
    padding-top: 12px;
    border-top: 1px solid var(--separator);
  }
  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--label-2);
  }
  details {
    padding-top: 12px;
    border-top: 1px solid var(--separator);
  }
  summary {
    cursor: default;
    font-size: 12px;
    font-weight: 600;
    color: var(--label-2);
  }
  .advanced {
    margin-top: 10px;
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
    font-variant-numeric: tabular-nums;
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
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .field {
    display: grid;
    grid-template-columns: minmax(96px, auto) 1fr;
    align-items: center;
    gap: 8px;
  }
  .field label {
    color: var(--label-2);
  }
  .range {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .range input[type="range"],
  .range select,
  .range input[type="number"] {
    flex: 1;
    min-width: 0;
  }
  .range .pct {
    width: 48px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .session {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .session-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .session select {
    width: 130px;
  }
  .group-chips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .group-chips li {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 10px;
    border-radius: 999px;
    background: var(--fill-strong);
    font-size: 12px;
  }
  .group-chips button {
    padding: 0 6px;
    line-height: 1.4;
  }
</style>
