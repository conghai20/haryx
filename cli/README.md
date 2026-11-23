# haryx

A unified control center for AI coding accounts, usage and sessions, from the command line.

`haryx` manages one person's own Claude Code logins on one machine: it stores each account, shows
every account's 5-hour and weekly usage, switches the live login in one command, runs Claude as a
given account per folder, and, if you turn it on, moves work to another of your accounts when the
active one reaches a threshold you set.

It is a Rust port of [claude-swap](https://github.com/realiti4/claude-swap) 1.2.0b1 with the
same commands, flags and `--json` output, so scripts written for `cswap` keep working (also without
the `ai` section). haryx keeps
its own data folder; `haryx ai import --from-cswap` copies an existing cswap setup across and leaves
cswap untouched.

## Use within Anthropic's terms

haryx is an independent project, not made, endorsed or supported by Anthropic. Use it only with
subscriptions you hold yourself, and never share a login. haryx sends no model requests: the official,
unmodified Claude Code does the work with each account's own login. Anthropic sets plan limits for
ordinary, individual use and may warn, throttle, suspend or close accounts it believes break its
[Consumer Terms](https://www.anthropic.com/legal/consumer-terms),
[Usage Policy](https://www.anthropic.com/legal/aup) or Claude Code
[legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) rules; you are responsible
for how you use your accounts. Don't use `haryx ai auto` to get more out of a plan than the plan allows.

## Install

Download `haryx-v<version>-<target>.tar.gz` from
[GitHub Releases](https://github.com/conghai20/haryx/releases), or the macOS app's DMG, which bundles
`haryx` and offers to put it on your PATH. To build it:

```sh
cargo install --locked --git https://github.com/conghai20/haryx haryx
```

Homebrew, crates.io, PyPI and npm packages are coming.

## Use

```sh
haryx ai add                  # store the current Claude Code login
haryx ai list                 # every account with its usage
haryx ai switch               # rotate to the next account (--strategy best picks by quota)
haryx ai switch 2             # switch to account 2 (number, email or alias)
haryx ai run 2 -- --resume    # run Claude as account 2 in this terminal only
haryx ai map 2                # always run as account 2 in this folder
haryx ai auto                 # opt-in: move to another of your accounts at your threshold
haryx ai tui                  # interactive dashboard (bare `haryx` in a terminal)
haryx ai login 3              # sign account 3 in again ("re-login needed")
haryx ai profile show         # your Claude Code setup: skills, hooks, MCPs, plugins, history
haryx ai profile export f.haryx-profile        # move it to another machine...
haryx ai profile import work f.haryx-profile --apply   # ...and apply it there (backup first)
haryx ai wake on 1 && haryx ai wake install    # start account 1's next 5h window right after each reset
haryx ai help                 # every AI-account command
haryx help                    # the sections
```

Account, usage and session commands live in the `ai` section. The cswap spellings without it
(`haryx list`, `haryx switch 2`, `haryx --list`) keep working; `login`, `profile` and `wake` are the exceptions,
since their top-level names are kept for HaryX itself. Profiles are described in
[docs/ai-profile-plan.md](../docs/ai-profile-plan.md), wake in [docs/ai-wake-plan.md](../docs/ai-wake-plan.md).

Data lives in `~/Library/Application Support/HaryX` on macOS and `$XDG_DATA_HOME/haryx` on Linux;
set `HARYX_HOME` to choose another folder. Windows is on the roadmap.

## License

MIT. See `LICENSE`; portions are ported from claude-swap (MIT, Onur Cetinkol).
