# haryx

A unified control center for AI coding accounts, usage and sessions, from the command line.

`haryx` manages one person's own Claude Code logins on one machine: it stores each account, shows
every account's 5-hour and weekly usage, switches the live login in one command, runs Claude as a
given account per folder, and, if you turn it on, moves work to another of your accounts when the
active one reaches a threshold you set.

It is a Rust port of [claude-swap](https://github.com/realiti4/claude-swap) 1.2.0b1 with the
same commands, flags and `--json` output, so scripts written for `cswap` keep working. haryx keeps
its own data folder; `haryx import --from-cswap` copies an existing cswap setup across and leaves
cswap untouched.

## Use within Anthropic's terms

haryx is an independent project, not made, endorsed or supported by Anthropic. Use it only with
subscriptions you hold yourself, and never share a login. haryx sends no model requests: the official,
unmodified Claude Code does the work with each account's own login. Anthropic sets plan limits for
ordinary, individual use and may warn, throttle, suspend or close accounts it believes break its
[Consumer Terms](https://www.anthropic.com/legal/consumer-terms),
[Usage Policy](https://www.anthropic.com/legal/aup) or Claude Code
[legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) rules; you are responsible
for how you use your accounts. Don't use `haryx auto` to get more out of a plan than the plan allows.

## Install

```sh
brew install conghai20/tap/haryx     # macOS, Linux
cargo install --locked haryx         # from source
pipx install haryx                   # or: uv tool install haryx
npm install --global haryx
```

The HaryX menu bar app for macOS bundles the same binary.

## Use

```sh
haryx add                  # store the current Claude Code login
haryx list                 # every account with its usage
haryx switch               # rotate to the next account (--strategy best picks by quota)
haryx switch 2             # switch to account 2 (number, email or alias)
haryx run 2 -- --resume    # run Claude as account 2 in this terminal only
haryx map 2                # always run as account 2 in this folder
haryx auto                 # opt-in: move to another of your accounts at your threshold
haryx tui                  # interactive dashboard (bare `haryx` in a terminal)
haryx help                 # everything else
```

Data lives in `~/Library/Application Support/HaryX` on macOS and `$XDG_DATA_HOME/haryx` on Linux;
set `HARYX_HOME` to choose another folder. Windows is on the roadmap.

## License

MIT. See `LICENSE`; portions are ported from claude-swap (MIT, Onur Cetinkol).
