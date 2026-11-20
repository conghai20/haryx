# contracts

Language-neutral data contracts shared by every HaryX part (macOS app, `haryx` CLI, and later server, web, iOS,
Android). Add or change a shape here first, then update the code that reads or writes it.

| Path | What | Checked by |
|---|---|---|
| `cswap/list.json` | `cswap list --json` / `haryx list --json` payload (schemaVersion 1, from cswap 1.1.0, anonymised) | `apps/macos/Tests/HaryXCoreTests` |

The Ruby CLI must produce output that matches these fixtures (drop-in compatibility with cswap).
