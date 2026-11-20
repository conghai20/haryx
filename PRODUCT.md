# Product

<!-- impeccable:product-schema 1 -->

> Facts marked *(inferred)* came from the repository and the owner's brief, not from an interview answer.
> Confirm or correct them before they steer major decisions.

## Platform

macos

Two parts:
- **HaryX app:** native macOS (SwiftUI, macOS 26+, Liquid Glass), menu bar status item plus dashboard window.
  Native macOS conventions apply. *(Platform value is outside the web/ios/android/adaptive set; recorded as-is.)*
- **`haryx` CLI:** Ruby, cross-platform (macOS, Linux, Windows). The terminal is its interface.
- **Future:** a server (language undecided), a web dashboard, and native iOS and Android apps. The CLI keeps
  credentials on the device; the server receives metadata only. Layout: `docs/architecture.md`.

## Users

Developers who run several AI coding accounts (today: Claude accounts) and need to see quota, switch accounts and
track live sessions without leaving their flow. *(inferred profile)* HaryX is **public from the start**: first-time
users who have never used `cswap` must be able to install and onboard without reading the upstream docs.

## Product Purpose

**HARYX: take control of your AI workflow and other DevSecOps, SRE, Cloud… all in one.**

Slogan: *A unified control center for AI coding accounts, usage and sessions.*

Today it is a control center for Claude accounts: live 5-hour/weekly usage, switching (manual, best, next,
auto-switch), account management, sessions running per account, terminal launch, and warnings before quota runs out.
The stated ambition extends the same control-center role to DevSecOps, SRE and Cloud workflows; none of that
exists yet and its scope is undecided.

Success *(inferred)*: the user never hits an exhausted account by surprise and never needs the terminal to manage
accounts.

## Positioning

One place that sees every AI coding account, its quota and the sessions running on it, and routes work between them.
The switching/routing idea is the core of the identity (it is what the X mark represents).

## Operating Context

- Lives in the macOS menu bar; glanced at mid-task, opened briefly, closed.
- Users already work in terminals (Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty) and launch Claude
  sessions from them.
- Data comes from the account-switching CLI (`cswap list --json`, `cswap auto --json`, etc.) and a native process
  scan of running `claude` processes.

## Capabilities and Constraints

- Current: every `cswap` command (switch, add, add-token, remove, enable/disable, alias, threshold, priority, move,
  swap, map/unmap, run, auto, config, export/import, import-usage, unclaimed, upgrade, purge), notifications,
  terminal quick actions, launch at login.
- Zero third-party Swift dependencies; SwiftPM build; ad-hoc signed `.app`.
- Planned: fork upstream claude-swap (Python) and rebuild its full functionality in Ruby as the **`haryx`** CLI.
  It is **drop-in compatible**: same commands, flags, prompts and `--json` output shapes as `cswap`, so the app only
  changes the binary it calls. HaryX-only commands may be added later without breaking that contract.
  Open: upstream license terms (check before forking). Windows support is on the roadmap (`docs/roadmap.md`).
- Renaming from `cswapui` to HaryX (app name, bundle, binary, docs) is planned.
- Distribution (all wanted): Homebrew (formula for the CLI, cask for the app), RubyGems (`gem install haryx`),
  Developer ID signed and notarized DMG via GitHub releases, plus wrapper packages on other managers (pip, npm and
  similar) so the CLI installs on macOS, Linux and Windows. How the non-Ruby wrappers ship a Ruby runtime is open.
- Undecided: which DevSecOps/SRE/Cloud area comes after AI accounts. Focus stays on AI accounts until the core ships.

## Brand Commitments

Binding, set by the owner:

- **Name:** HaryX in running text; **HARYX** as the uppercase wordmark.
- **Mark:** the letter X built from **two continuous geometric strokes** that intersect or flow to form the X.
  It stands for three things at once: Aquarius (fluidity, independence, unconventional thinking), X
  (experimentation, switching, connectivity), and flow (continuous movement between AI accounts, sessions and
  workflows), expressed through geometry only. Chosen direction: **"Wing H"**. On a midnight navy rounded-square tile, two point-symmetric
  halves (Electric Cyan above, Soft White below) interlock as a fluid X. In each half, one arm rises into a
  **square-cut vertical stem** and the other sweeps into a **pointed wing tail** that ends on the same top line.
  The four ends form the legs of an **H** (Haryo, HARYX), and the narrow diagonal gap between the halves is its
  crossbar. Curves are continuous with no kinks. Flat vector: no gradients, glows, shadows or metallic
  effects. No zodiac art, droplets, robots, brains or circuit patterns. Must stay recognizable at 16, 32, 64 and
  256 px. Refinement brief and variant status: `docs/brand/logo-refinement.md`.
- **Palette:** Midnight Navy `#101827`, Electric Cyan `#22D3EE`, Soft White `#F1F5F9`. Flat, solid colors only.
- **Typography:** Space Grotesk primary, Geist alternative; wordmark is custom geometric uppercase lettering
  (confident, clean, slightly futuristic, not sci-fi).
- **Logo use:** symbol only for app icon, Dock, Windows icon and favicon; symbol + HARYX for product and GitHub
  branding; monochrome version for terminal and documentation. Versions required: primary (dark), light,
  monochrome, symbol only, symbol + wordmark lockup.
- Pixel mascots (8 original species, one per account) stay, **restyled to the HaryX palette**
  (Midnight Navy / Electric Cyan / Soft White).
- *(inferred)* HaryX is independent: not affiliated with Anthropic or the upstream claude-swap project.

## Evidence on Hand

- `README.md`, `PLAN.md`: feature and architecture description.
- `Tests/CSwapCoreTests`: core test suite (12 tests passing).
- Measured performance: ~0.3% CPU idle; RSS observed ~85 MB just after launch, against a README claim of ~20 MB
  (unverified after settle).
- No users, testimonials, screenshots, press or benchmarks exist. Do not fabricate them.

## Product Principles

1. **Glance first.** The menu bar answers "which account, how much left, is anything running" without opening anything.
2. **Never surprised by quota.** Warn early, switch before exhaustion, explain what auto-switch did.
3. **Routing is the product.** Switching between accounts and sessions should feel precise and deliberate, like the mark.
4. **Native and light.** Feel like a first-party macOS utility; small footprint, no idle cost.
5. **Confirm before destroying.** Remove, purge and overwrite always ask first.

## Accessibility & Inclusion

Honor Reduce Motion (mascots go static). No other product-specific requirement established.
