<script lang="ts">
  import { onMount } from "svelte";
  import { auth } from "./lib/auth.svelte";
  import State from "./components/State.svelte";
  import Placeholder from "./views/Placeholder.svelte";
  import Privacy from "./views/Privacy.svelte";

  const sections = [
    { id: "machines", label: "Machines", blurb: "Where HaryX is signed in.", task: "task 17" },
    { id: "accounts", label: "Accounts", blurb: "Your Claude accounts across machines.", task: "task 19" },
    { id: "usage", label: "Usage", blurb: "Per account, per day, all machines.", task: "task 19" },
    { id: "profiles", label: "Profiles", blurb: "Synced profile files.", task: "task 19" },
    { id: "privacy", label: "Privacy center", blurb: "", task: "" },
  ] as const;

  type Id = (typeof sections)[number]["id"];
  const fromHash = (): Id => sections.find((s) => s.id === location.hash.slice(1))?.id ?? "machines";
  let current = $state<Id>(fromHash());
  const active = $derived(sections.find((s) => s.id === current)!);

  onMount(() => {
    const onHash = () => (current = fromHash());
    addEventListener("hashchange", onHash);
    void auth.init();
    return () => removeEventListener("hashchange", onHash);
  });
</script>

{#if auth.status === "starting"}
  <main class="solo"><State kind="loading" /></main>
{:else if auth.status === "signedOut" || auth.status === "failed"}
  <main class="solo">
    {#if auth.status === "failed"}
      <State kind="error" title="Could not reach WeCoAp sign-in" onAction={() => auth.init()} />
    {:else}
      <State kind="signedOut" onAction={auth.signIn} />
    {/if}
  </main>
{:else}
  <div class="shell">
    <nav aria-label="Sections">
      <strong class="brand">HaryX</strong>
      <ul>
        {#each sections as s (s.id)}
          <li><a href={`#${s.id}`} aria-current={s.id === current ? "page" : undefined}>{s.label}</a></li>
        {/each}
      </ul>
      <div class="user">
        <span title={auth.email ?? ""}>{auth.email ?? "Signed in"}</span>
        <button onclick={auth.signOut}>Sign out</button>
      </div>
    </nav>
    <main>
      {#if active.id === "privacy"}
        <Privacy />
      {:else}
        <Placeholder title={active.label} blurb={active.blurb} task={active.task} />
      {/if}
    </main>
  </div>
{/if}

<style>
  .solo, main { padding: 1.5rem 1rem; }
  .shell { display: grid; grid-template-columns: 14rem 1fr; min-height: 100vh; }
  nav { background: var(--surface); border-right: 1px solid var(--separator); padding: 1rem; display: flex; flex-direction: column; gap: 1rem; min-width: 0; }
  .brand { font-size: 1.1rem; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: .15rem; }
  li a { display: block; padding: .5rem .7rem; border-radius: var(--radius-control); color: var(--label); text-decoration: none; }
  li a:hover { background: var(--fill); }
  li a[aria-current="page"] { background: var(--selected); }
  .user { margin-top: auto; display: grid; gap: .5rem; }
  .user span { color: var(--label-2); font-size: .85rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  main { min-width: 0; }
  @media (max-width: 640px) {
    .shell { grid-template-columns: 1fr; }
    nav { border-right: 0; border-bottom: 1px solid var(--separator); flex-direction: row; flex-wrap: wrap; align-items: center; }
    ul { grid-auto-flow: column; overflow-x: auto; max-width: 100%; order: 3; flex-basis: 100%; }
    li a { white-space: nowrap; }
    .user { margin: 0 0 0 auto; grid-auto-flow: column; align-items: center; max-width: 60%; }
  }
</style>
