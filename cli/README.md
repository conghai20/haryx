# haryx

A unified control center for AI coding accounts, usage and sessions, from the command line.

`haryx` manages several Claude Code logins on one machine: it stores each account, shows every
account's 5-hour and weekly usage, switches the live login in one command, runs Claude as a given
account per folder, and can switch automatically before a quota runs out.

It is a Rust port of [claude-swap](https://github.com/realiti4/claude-swap) 1.2.0b1 with the
same commands, flags and `--json` output, so scripts written for `cswap` keep working. haryx keeps
its own data folder; `haryx import --from-cswap` copies an existing cswap setup across and leaves
cswap untouched.

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
haryx auto                 # switch automatically near the threshold (--once for cron)
haryx tui                  # interactive dashboard (bare `haryx` in a terminal)
haryx help                 # everything else
```

Data lives in `~/Library/Application Support/HaryX` on macOS and `$XDG_DATA_HOME/haryx` on Linux;
set `HARYX_HOME` to choose another folder. Windows is on the roadmap.

## License

MIT. See `LICENSE`; portions are ported from claude-swap (MIT, Onur Cetinkol).
