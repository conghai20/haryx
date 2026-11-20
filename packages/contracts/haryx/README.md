# haryx JSON fixtures

Payloads captured from the Rust `haryx` CLI (`cli/`), checked by the macOS app tests so the app
keeps decoding haryx output as well as cswap's (`../cswap`).

- `list-unavailable.json` — `haryx list --json` in a sandbox with two accounts and the usage
  endpoint unreachable (`usage: null`, `usageError`, `usageRetryAt`).
