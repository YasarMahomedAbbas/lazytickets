# lazytickets

A lazygit-style terminal UI for **GitHub Projects v2** boards, built to run inside
tmux. It reads a board, shows issue detail, filters with presets + fuzzy search,
writes card status, and — the headline feature — drives the Claude Code session in
the project's `claude` tmux window to start work on a ticket.

> The crate and on-disk config are named **`lazytickets`**; the repo directory is
> `lazyissues`.

## Requirements

lazytickets is a thin, fast front-end over two tools it shells out to at runtime —
they are **not** bundled, so install them first:

- [`gh`](https://cli.github.com) — the GitHub CLI, authenticated (`gh auth login`).
  Status writes additionally need the `project` scope: `gh auth refresh -s project`.
- [`tmux`](https://github.com/tmux/tmux) — the app runs inside a tmux session; the
  start-work flow drives a `claude` window in it.

The binary preflight-checks for these on launch and tells you if one is missing.

## Install

### Quick install (Linux, no toolchain) — recommended

Downloads the right prebuilt binary for your machine into `~/.local/bin`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/YasarMahomedAbbas/lazytickets/releases/latest/download/lazytickets-installer.sh | sh
```

Prebuilt binaries are static musl builds for `x86_64` and `aarch64` Linux.
Override the location with `LAZYTICKETS_INSTALL_DIR`, or pin a version with
`LAZYTICKETS_VERSION=v0.2.0`.

### Download a tarball manually

Grab `lazytickets-<target>.tar.gz` from the
[latest release](https://github.com/YasarMahomedAbbas/lazytickets/releases/latest),
verify it against the `.sha256` sidecar, untar, and drop `lazytickets` on your PATH.

### From source with Cargo (any platform with Rust ≥ 1.85)

```sh
cargo install --git https://github.com/YasarMahomedAbbas/lazytickets   # latest tag
cargo install --path .                                                 # this checkout
```

This compiles locally, so it needs a Rust toolchain but works anywhere.

## Updating

- **curl install / tarball:** re-run the same curl one-liner — the installer always
  resolves `releases/latest`, so it overwrites your binary with the newest release.
- **Cargo:** `cargo install --git https://github.com/YasarMahomedAbbas/lazytickets --force`.

To check what you're running: `lazytickets --version` reflects the crate version the
binary was built from. Releases are announced in [`CHANGELOG.md`](./CHANGELOG.md).

## Usage

```sh
cargo run          # dev: launch the TUI (resolves the cwd's git repo → its board)
lazytickets        # installed
```

On first run in an unknown repo, an in-TUI wizard maps it to a project and writes
`~/.config/lazytickets/config.toml`. Press `?` for the full keybinding overlay.

## Releasing (maintainers)

Releases are automated and versioned with **[Semantic Versioning](https://semver.org)**
driven by [Conventional Commits](https://www.conventionalcommits.org). Two workflows
split the job:

1. **`.github/workflows/release-plz.yml`** — on every push to `master`,
   [release-plz](https://release-plz.dev) reads the commits since the last release and
   opens/updates a **Release PR** that bumps the version in `Cargo.toml` and regenerates
   `CHANGELOG.md`. Merging that PR creates the `vX.Y.Z` git tag and GitHub Release.
2. **`.github/workflows/release.yml`** — triggered by the new tag, cross-compiles the
   musl binaries, writes `.sha256` sidecars, and attaches them plus
   `lazytickets-installer.sh` to the release.

**Version bumps (pre-1.0):** `fix:` → patch, `feat:` → patch, and `feat!:` /
`BREAKING CHANGE:` → minor (`0.MINOR.x`). Bump to `1.0.0` when the app stabilises.

### One-time setup

The tag release-plz pushes must be created with a **Personal Access Token**, because
tags pushed by the default `GITHUB_TOKEN` do not trigger other workflows — so
`release.yml` would never fire. Create a fine-grained PAT (or classic PAT with `repo`)
and add it as the repo secret **`RELEASE_PLZ_TOKEN`**. No secret is needed for
`release.yml`; it uses the built-in `GITHUB_TOKEN`.

If you'd rather not use a PAT, you can cut a release by hand — push the tag yourself
(`git tag v0.2.0 && git push origin v0.2.0`) or run the **release** workflow via
*workflow_dispatch* with the tag; release-plz then just manages the changelog/version PR.

Crates.io publishing is off (`publish = false` in `release-plz.toml`) since we ship
binaries; flip it on and add `CARGO_REGISTRY_TOKEN` if you ever want it.

## Development

```sh
cargo build --release    # → target/release/lazytickets
cargo test               # pure, hermetic unit tests — no network, no gh/tmux
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

CI (`.github/workflows/ci.yml`) runs fmt + clippy + tests on every push and PR.
`PLAN.md` is the authoritative design doc; `CLAUDE.md` orients Claude Code in the
codebase.

## License

Dual-licensed under either of [Apache-2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT)
at your option.
