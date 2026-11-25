<script lang="ts">
  // The sign-in window: one `haryx ai login` run on the backend's pseudo-terminal, its sign-in link,
  // and a field for the code the sign-in page shows. Closing the window cancels the run.
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { api } from "../lib/api";
  import type { LoginView } from "../lib/types";

  const id = Number(new URLSearchParams(location.search).get("id"));
  /** Transcript lines shown under a failure. */
  const FAILURE_LINES = 6;

  let run = $state<LoginView | null>(null);
  let input = $state("");
  let copied = $state(false);
  let error = $state<string | null>(null);
  let outputOpen = $state(false);
  let outputEl = $state<HTMLPreElement | null>(null);

  let running = $derived(run?.state === "running");
  let status = $derived.by(() => {
    if (!run) return "Starting…";
    if (run.awaitingAnswer) return "haryx is asking a question below.";
    if (run.awaitingCode) return "Waiting for the code the sign-in page shows.";
    if (run.url) return "Waiting for you to sign in in the browser.";
    return "Starting Claude Code…";
  });
  let lastLines = $derived(
    (run?.text ?? "")
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l)
      .slice(-FAILURE_LINES)
      .join("\n"),
  );

  onMount(() => {
    const off = listen<LoginView>("login", (e) => {
      if (e.payload.id === id) run = e.payload;
    });
    api
      .loginGet(id)
      .then((v) => (run = v ?? run))
      .catch((e) => (error = String(e)));
    return () => {
      off.then((f) => f());
    };
  });

  $effect(() => {
    if (run?.state === "failed") outputOpen = true;
  });

  // Keep the newest output in view.
  $effect(() => {
    void run?.text;
    tick().then(() => outputEl?.scrollTo({ top: outputEl.scrollHeight }));
  });

  async function attempt(action: () => Promise<unknown>) {
    error = null;
    try {
      await action();
    } catch (e) {
      error = String(e);
    }
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const line = input.trim();
    if (!line) return;
    attempt(async () => {
      await api.loginSend(id, line);
      input = "";
    });
  }

  const copy = (text: string) => attempt(() => navigator.clipboard.writeText(text));
  const close = () => attempt(() => getCurrentWindow().close());
</script>

<main class="login">
  <header>
    {#if running}<span class="spinner" aria-hidden="true"></span>{/if}
    <div>
      <h1>{run?.title ?? "Sign In"}</h1>
      <p class="secondary">
        Claude Code is signing you in. Your browser should open; sign in there with your own account.
      </p>
    </div>
  </header>

  {#if !run || running}
    <p class="status" role="status">{status}</p>

    {#if run?.url}
      <section class="box" aria-label="Sign-in link">
        <p class="caption secondary">Sign-in link</p>
        <p class="link" title={run.url}>{run.url}</p>
        <div class="row">
          <button onclick={() => attempt(() => api.loginOpen(id))}>Open in browser</button>
          <button
            onclick={async () => {
              await copy(run?.url ?? "");
              copied = !error;
            }}>{copied ? "Copied" : "Copy link"}</button
          >
        </div>
        <p class="caption tertiary">Only open this link yourself. Never send it to someone else to sign in for you.</p>
      </section>
    {/if}

    <form class="box" onsubmit={submit}>
      <label for="code" class="caption secondary">
        {run?.awaitingAnswer ? "Answer (y or n)" : "If the sign-in page shows a code, paste it here"}
      </label>
      <div class="row">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          id="code"
          class="code"
          autocomplete="off"
          spellcheck="false"
          autofocus
          placeholder={run?.awaitingAnswer ? "y / n" : "Code"}
          bind:value={input}
          disabled={!run}
        />
        <button class="primary" type="submit" disabled={!run || !input.trim()}>Submit</button>
      </div>
    </form>
  {:else if run.state === "succeeded"}
    <p class="outcome ok" role="status">✓ {run.message}</p>
  {:else if run.state === "failed"}
    <div class="outcome failed" role="alert">
      <p>⚠ {run.message}</p>
      {#if lastLines}<pre>{lastLines}</pre>{/if}
    </div>
  {:else}
    <p class="outcome secondary" role="status">{run.message}</p>
  {/if}

  {#if error}<div class="banner error" role="alert">{error}</div>{/if}

  <details bind:open={outputOpen}>
    <summary class="secondary">Terminal output</summary>
    <pre bind:this={outputEl}>{run?.text || "Starting…"}</pre>
  </details>

  <footer class="row">
    <button onclick={() => copy(run?.text ?? "")}>Copy output</button>
    <span class="spacer"></span>
    {#if running}
      <button onclick={() => attempt(() => api.loginCancel(id))}>Cancel</button>
    {:else}
      <button class="primary" onclick={close}>Done</button>
    {/if}
  </footer>
</main>

<style>
  .login {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    overflow-y: auto;
    user-select: text;
  }

  header {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }

  h1 {
    margin: 0 0 2px;
    font-size: 16px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  header p,
  .status {
    margin: 0;
  }

  .status {
    color: var(--cyan-text);
  }

  .spinner {
    flex: none;
    width: 14px;
    height: 14px;
    margin-top: 3px;
    border: 2px solid var(--track);
    border-top-color: var(--cyan);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-card);
    background: var(--surface);
    border: 1px solid var(--separator);
  }

  .box p {
    margin: 0;
  }

  .link {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 11px;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .code {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
  }

  .outcome {
    margin: 0;
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  .outcome.ok {
    color: var(--cyan-text);
  }

  .outcome.failed p {
    margin: 0 0 6px;
    color: var(--amber);
  }

  pre {
    margin: 0;
    padding: 8px;
    max-height: 140px;
    overflow: auto;
    border-radius: var(--radius-control);
    background: var(--fill);
    font: 11px/1.4 ui-monospace, "Cascadia Mono", Consolas, monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  details summary {
    cursor: default;
    margin-bottom: 6px;
  }

  footer {
    margin-top: auto;
  }

  .spacer {
    flex: 1;
  }
</style>
