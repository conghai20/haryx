# Plan: `haryx ai profile` — move a Claude Code setup between machines

Status: planned. Branch base: `feat/ai-namespace`.

## Problem

One person works on several machines. Each machine's Claude Code setup (skills, hooks, MCP servers,
plugins, rules, agents, commands, `settings.json`) drifts, because nothing carries it across. Accounts
are not the cause: on one machine every account already shares `~/.claude`.

`haryx ai profile` lets the user see that setup, save it under a name, export it to one file, and import
and apply that file on another machine. No server and no git repository are needed; the file can travel
by AirDrop, USB, or any file share.

## Decisions

| Topic | Decision |
|---|---|
| Command | `haryx ai profile …`, its own command. Not under `haryx ai config` (that is the cswap-compatible haryx settings command) and not `haryx ai export/import` (those move accounts and credentials). |
| `default` | Always the live `~/.claude`. Reserved: it cannot be saved, imported, or deleted under that name. |
| Import | Stores the profile only. `apply` writes it into `~/.claude`; `import --apply` does both. |
| Apply mode | Merge: add and update, keep local-only items. `--prune` also removes local items the profile does not have (setup categories only, never history). |
| History | Exported by default (`--no-history` to leave it out): conversations, prompt history, file checkpoints, todos, plans. |
| Secrets | Chosen per export: `--secrets redact` (default) or `--secrets encrypt` (age, passphrase). Never plaintext. |
| Plugins | Chosen per export: `--plugins ref` (default) or `--plugins bundle`. |
| Third-party folders | Opt-in per export: `--include-dir PATH` (repeatable), e.g. `~/.claude/ecc`, `~/.claude-mem`. |
| Encryption | Rust `age` crate, scrypt passphrase recipient; files decrypt with the `age` CLI too. |

### Naming

`haryx run` already has per-account "profiles" under `<store>/sessions/`. From now on code and docs call
those **session profiles**; the bare word **profile** means a setup profile from this plan.

## Commands

```
haryx ai profile list                                   # saved profiles; marks the last applied one
haryx ai profile show [NAME]                            # inventory; NAME defaults to "default" (live ~/.claude)
haryx ai profile save NAME                              # snapshot live ~/.claude as NAME
haryx ai profile export [NAME] FILE                     # write a portable .haryx-profile file
      [--secrets redact|encrypt] [--plugins ref|bundle] [--no-history]
      [--only CATS] [--exclude CATS] [--include-dir PATH]...
haryx ai profile import NAME FILE [--dry-run] [--apply] [--force]
haryx ai profile apply NAME [--dry-run] [--yes] [--prune] [--no-history]
haryx ai profile diff NAME [OTHER]                      # OTHER defaults to "default"
haryx ai profile delete NAME
haryx ai profile rollback [BACKUP_ID]                   # undo the last (or a given) apply
```

- Every read command takes `--json`; shapes land in `packages/contracts/haryx/` first (contracts-first rule).
- `export` with no NAME exports `default`.
- `import` refuses an existing NAME unless `--force`.
- `CATS` is a comma list of categories: `settings,hooks,skills,agents,commands,rules,output-styles,plugins,mcps,memory,history`.
- `--no-history` is the same as `--exclude history`; in the macOS app it is an "Include history" box,
  checked by default.

## Inventory (`show`)

| Category | Source | Notes |
|---|---|---|
| settings | `~/.claude/settings.json` | permissions, env, model, statusLine, `enabledPlugins`; `hooks` key is listed under hooks |
| hooks | `settings.json` `hooks`; scripts they reference (`~/.claude/hooks`, `~/.claude/scripts`); plugin hooks | each entry: event, matcher, command, source |
| skills | `~/.claude/skills`; plugin skills | source marked `user` or `plugin:<name>` |
| agents, commands, rules, output-styles | their folders under `~/.claude` | |
| plugins | `plugins/installed_plugins.json`, `plugins/known_marketplaces.json` | name, marketplace, version, enabled |
| mcps | `mcpServers` in `~/.claude.json` (user scope); plugin MCPs; `~/.claude/mcp-configs/` | project-scope `.mcp.json` files are out of scope |
| memory | `CLAUDE.md`, `AGENTS.md`, `keybindings.json` | |
| history | `projects/` (conversation transcripts and per-project memory), `history.jsonl` (prompt history), `file-history/` (edit checkpoints), `todos/`, `plans/` | on by default; `show` lists counts and size, not every file |

Item status: `ok`, `broken` (missing hook script, MCP command not on `PATH`), `needs-secret` (placeholder
without a value), `secret` (contains a detected secret; export will redact or encrypt it).

**Never exported, with no option:**

- Credentials: `.credentials.json`, Keychain items, the haryx store. Accounts move with `haryx ai export`.
- Anthropic-managed signed files: `policy-limits*`, `remote-settings*`. They are per account and refreshed
  by Claude Code; a copy from another machine would be stale or rejected.
- Machine-local runtime state: `sessions/` (records of running processes, keyed by PID), `session-env/`,
  `shell-snapshots/`, `ide/` (lock files of open editors), `cache/`, `debug/`, `backups/`, `statsig/`,
  `paste-cache/`. It means nothing on another machine. The conversations themselves are in `projects/`,
  which the history category carries.
- Every `~/.claude.json` key besides `mcpServers` (account identity, folder trust decisions, machine state).

## Storage

```
<store>/profiles/<NAME>/manifest.json
<store>/profiles/<NAME>/files/…
<store>/profiles/backups/<UTC-timestamp>/   # written before every apply
```

`<store>` is the haryx data folder (`~/Library/Application Support/HaryX`, `$XDG_DATA_HOME/haryx`, or
`HARYX_HOME`). Directories `0700`, files `0600`.

## Export file

`NAME.haryx-profile`: a gzip tar containing `manifest.json` and `files/`.

`manifest.json`:

```json
{
  "schema": 1,
  "kind": "haryx-profile",
  "name": "work",
  "haryx_version": "0.x.y",
  "created_at": "2026-11-23T15:00:00Z",
  "source": { "os": "macos", "arch": "aarch64" },
  "options": { "secrets": "redact", "plugins": "ref" },
  "items": [
    { "category": "skills", "id": "skills/review", "path": "files/skills/review", "sha256": "…" },
    { "category": "plugins", "id": "caveman@caveman-marketplace", "version": "1.2.0", "mode": "ref" },
    { "category": "mcps", "id": "github", "secrets": ["GITHUB_TOKEN"] }
  ],
  "include_dirs": [ { "origin": "${HOME}/.claude-mem", "path": "files/extra/claude-mem" } ],
  "requires": ["node", "uv", "jq"],
  "secrets": { "mode": "redact", "names": ["GITHUB_TOKEN"] }
}
```

- **Home paths** in text items (hook commands, MCP commands and args, settings) are rewritten from the
  absolute home to `${HOME}` on export and expanded on apply.
- **`requires`**: executables found in hook and MCP commands. Import reports the ones missing on `PATH`.
- **Secrets, redact**: detected values in MCP `env`/`headers`, settings `env`, and included files are
  replaced by `${HARYX_SECRET_<NAME>}`. On apply, values come from the environment variable of the same
  name, then the Keychain, then a prompt; with `--yes` and no value the item is applied and marked
  `needs-secret`.
- **Secrets, encrypt**: the secret values go into `secrets.age` inside the archive, encrypted with a
  passphrase; everything else stays readable so `--dry-run` works without the passphrase.
- **Plugins, ref**: name, marketplace URL, version. Apply runs `claude plugin marketplace add` and
  `claude plugin install`.
- **Plugins, bundle**: the plugin's files are copied too; apply uses them only when the marketplace
  cannot be reached or the version is gone.
- **History** is stored under `files/history/`. Project folders in `projects/` are named after the
  project path (`-Users-me-code-app`); on apply, the source home prefix in those names and in each
  transcript's `cwd` field is rewritten to the target home. Projects outside the home keep their path.
- **Secrets in history**: transcripts can contain pasted keys. With `redact`, detected values are replaced
  by `[REDACTED:haryx]` (not restorable). With `encrypt`, the whole history part is encrypted as
  `history.tar.age`.
- **Size**: no fixed cap on export; `export` prints the size per category and warns above 500 MB. The
  reader refuses archives whose extracted size exceeds the manifest's declared total (zip-bomb guard).

Secret detection reuses `credentials::looks_like_api_key` and adds patterns for `sk-ant-`, GitHub
(`ghp_`, `github_pat_`), Slack, AWS, generic `*_TOKEN`/`*_KEY`/`*_SECRET` env names, and bearer headers.

## Import and apply

1. **Read** the archive as untrusted input: reject absolute paths, `..`, symlinks, device files, and
   anything over the size cap; verify each `sha256` against the manifest.
2. **Plan** each item against the live `~/.claude`: `add`, `update`, `same`, `conflict` (changed on both
   sides since a common base is unknown; shown as a diff), `skip` (excluded or unsupported on this OS).
   `--dry-run` prints the plan and stops. With `--prune`, local items of the included setup categories
   that the profile lacks are planned as `remove`.

   History merges without loss: transcripts missing locally are added; a transcript present on both
   sides keeps the longer one (transcripts only grow) and the other is kept in the backup;
   `history.jsonl` is the union of both, de-duplicated and ordered by timestamp. `--prune` never
   removes history.
3. **Confirm** — the import runs code. Before any write, list every hook command and every MCP command
   with its arguments and require a `y`, unless `--yes`.
4. **Back up** every path about to change into `<store>/profiles/backups/<timestamp>/` with a manifest.
5. **Write**: files atomically (`fsutil::atomic_write_text`); `settings.json` merged per key (hooks merged
   per event and matcher); `mcpServers` merged into `~/.claude.json` under the Claude config lock
   (`claude_locks`), touching no other key; plugins via the `claude` CLI.
6. **Report** what changed, what is `needs-secret`, which `requires` are missing, and warn when Claude
   sessions are running (they pick up changes only after restart).

`rollback` restores a backup in the same way (backup of the current state first, then restore).

Cross-OS: macOS-only paths or tools on Linux produce warnings, not failures. Windows follows the
existing Windows roadmap.

## Security

- An imported profile is code: hooks and MCP servers execute on the user's machine. Step 3 is mandatory
  for interactive use.
- Secrets never leave the machine in plaintext. `export` refuses to write a detected secret when the
  chosen mode cannot cover it (for example inside an `--include-dir` binary file); the user must
  `--exclude` it.
- The archive is untrusted: path traversal, symlink, and size checks before extraction.
- No network use besides the `claude` CLI installing plugins.

## Code layout

- `haryx-core/src/profile/` — new module:
  - `inventory.rs` — scan live `~/.claude` or a stored profile into items.
  - `secrets.rs` — detection, redaction, `age` encryption.
  - `archive.rs` — tar.gz write and safe read, manifest, hashes.
  - `plan.rs` — item diff and apply plan.
  - `apply.rs` — backup, merge writes, plugin install, rollback.
  - `mod.rs` — `ProfileStore` (list/save/delete under `<store>/profiles`).
- `haryx/src/cli/` — `Profile` command and `ProfileAction` in `mod.rs`, parser in `subparsers.rs`, help,
  output formatting; wired into the `ai` section.
- New crates: `tar`, `flate2`, `sha2`, `age` (phase 4 only).
- `packages/contracts/haryx/` — `profile-show.json`, `profile-list.json`, `profile-plan.json`.

## Phases

Each phase is test-first (unit tests in `haryx-core`, end-to-end tests in `crates/haryx/tests/` against a
temporary `HOME` and `HARYX_HOME`) and keeps `cargo test --workspace` and clippy green.

1. **`show` and `list`** — inventory of live `~/.claude`, item status, `--json` contract. Read-only.
2. **`save` and `export`** — stored profiles, archive writer, `${HOME}` rewriting, `--secrets redact`,
   `--plugins ref`, `--only/--exclude`, `requires`.
3. **`import`, `apply`, `diff`, `rollback`, `delete`** — safe archive reader, plan, confirmation, backup,
   merge writes, plugin install, `needs-secret`.
4. **`--secrets encrypt`, `--plugins bundle`, `--include-dir`.**
5. **macOS app** — a Profiles section: inventory with status badges, export and import sheets with the
   dry-run plan and the hook/MCP confirmation.

## Open questions

- Should `show default` also list project-scope `.mcp.json` of the current folder? Out of scope for now.
