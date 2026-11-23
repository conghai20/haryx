# Releasing HaryX

One tag builds every artifact; publishing the resulting draft release pushes to the registries.

## Channels

| Channel | What ships | Built by | Source |
| --- | --- | --- | --- |
| GitHub Releases | `haryx-v<ver>-<target>.tar.gz` (+ `.sha256`) for `aarch64`/`x86_64` macOS and Linux (musl, static) | `release.yml` → `binaries` | `scripts/release/package.sh` |
| Homebrew | `haryx.rb` formula installing the release tarballs | `release.yml` → `release` | `scripts/release/formula.sh` |
| crates.io | `haryx-core`, then `haryx` (`cargo install --locked haryx`) | `publish.yml` → `crates` | `cli/` |
| PyPI | binary-only wheels per platform (`pipx`/`uv tool install haryx`) | `release.yml` → `wheels`, `publish.yml` → `pypi` | `packaging/pypi` (maturin) |
| npm | `haryx`: a shim that downloads the release binary for the platform and verifies its SHA-256 | `publish.yml` → `npm` | `packaging/npm` |
| DMG | `HaryX-<app ver>.dmg`: the menu bar app with `haryx` bundled in `Contents/Resources/bin`; the app offers to link it onto the PATH | `release.yml` → `dmg` | `apps/macos/scripts/package-dmg.sh` |
| Homebrew cask (planned) | the DMG's app; `binary "#{appdir}/HaryX.app/Contents/Resources/bin/haryx"` links the CLI | — | tap repo `Casks/haryx.rb` |

npm 12 no longer runs install scripts by default, so the npm shim downloads the binary on its first
run when `postinstall` was skipped. The app finds the bundled CLI last, so a Homebrew/cargo/pip
install always wins and stays upgradable with `haryx upgrade`.

The DMG installs the CLI too: on first launch the app offers to link the bundled `haryx` into
`/usr/local/bin` (admin prompt) or `~/.local/bin` (no prompt); Preferences › Terminal command does
the same later. It is a symlink into the app, so updating the app updates the command, and it never
replaces a `haryx` installed another way. A future cask's `binary` stanza links the same file.

## Steps

1. Move the `## [Unreleased]` entries of `CHANGELOG.md` under `## [<version>] - <date>` (Breaking
   changes first) and add its compare link. The release notes are that section
   (`scripts/release/notes.sh <version>`); the release fails without it.
2. Bump the version in `cli/Cargo.toml` (`[workspace.package]`) and `packaging/npm/package.json`
   (CI refuses a mismatch); bump `VERSION` in `apps/macos/scripts/build.sh` for an app release.
3. `git tag v<version> && git push origin v<version>`.
4. Wait for `Release`; review the draft release and its assets.
5. Publish the draft. `Publish` then runs the registries that are switched on.
6. Copy `haryx.rb` from the release into the tap repo (`conghai20/homebrew-tap`, `Formula/haryx.rb`).

## One-time setup

- **Repository variables** (Settings → Variables → Actions): `PUBLISH_CRATES`, `PUBLISH_PYPI`,
  `PUBLISH_NPM` = `true` to enable each channel. Unset means skipped.
- **crates.io:** `CARGO_REGISTRY_TOKEN` secret (a token scoped to publish `haryx`, `haryx-core`).
- **PyPI:** trusted publishing — add this repo, workflow `publish.yml`, environment `pypi` as a
  publisher of project `haryx` on pypi.org. No token is stored.
- **npm:** `NPM_TOKEN` secret (automation token).
- **Apple signing (optional):** `MACOS_CERT_P12` (base64 Developer ID Application `.p12`),
  `MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY` (`Developer ID Application: Name (TEAMID)`); for
  notarization also `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` (app-specific password).
  Without them the DMG is ad-hoc signed and Gatekeeper warns on other Macs.
- **Homebrew tap:** create `conghai20/homebrew-tap` once; users then `brew install conghai20/tap/haryx`.

## Local builds

```sh
cd cli && cargo build --release -p haryx                  # binary
scripts/release/package.sh aarch64-apple-darwin cli/target/release/haryx   # release tarball
(cd packaging/pypi && uvx maturin build --release)         # wheel
(cd packaging/npm && npm pack)                             # npm tarball
apps/macos/scripts/package-dmg.sh                          # DMG (SIGN_IDENTITY / NOTARY_PROFILE optional)
```

`haryx`'s own update check stays off until releases exist (`update_check::RELEASES_URL`).
