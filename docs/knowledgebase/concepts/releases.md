---
type: Concept
title: Releases and packaging
description: How tower releases work — tag push, CI build matrix, GitHub release, Homebrew tap, and AUR.
tags:
  - releases
  - packaging
  - ci
  - homebrew
  - aur
status: stable
generated:
  by: claude-code/1.0
  at: "2026-10-17T00:00:00Z"
sources:
  - resource: github:geoffjay/nemo/.github/workflows/release.yml
    title: nemo release workflow (precedent)
  - resource: github:geoffjay/horde/.goreleaser.yml
    title: horde goreleaser config (precedent)
---

# Releases and packaging

Releases are driven by `.github/workflows/release.yml`, triggered by `v*`
tag pushes (or manual `workflow_dispatch` with a tag). The process is fully
automated: tag → CI gate → cross-platform build → GitHub release → Homebrew
formula → AUR PKGBUILD.

Cites [design §3](design/03-process-and-deployment.md) (deployment model:
single `tower` binary, stateless clients).

# Release process

One command, from a clean `main`:

```
cargo make release -- --version 0.4.0   # becomes tag v0.4.0
```

`scripts/release.sh` does all the steps:

1. Preflight: `git`, `gh`, `git-cliff`, `cargo` are installed (`cargo install
   git-cliff`); the tree is clean; `main` is fast-forwarded; the tag and the
   `release/v0.4.0` branch don't exist yet; the version is valid semver.
2. Bumps `workspace.package.version` in the root `Cargo.toml` (all crates
   inherit it; internal crates are `workspace = true` path deps with no
   pinned version, so this is the only line that changes).
3. Gates: `cargo make lint`, `cargo make test`, and checks that
   `tower --version` reports the new version.
4. Regenerates `CHANGELOG.md` with git-cliff (`cliff.toml`: conventional
   commits grouped into Added / Fixed / Changed / Documentation / Tests /
   Chore; merges and release commits skipped).
5. Commits `chore: release v0.4.0` on `release/v0.4.0`, opens a PR with the
   release notes as its body, and watches the PR's checks.
6. Waits for the PR to be merged (the operator merges), then tags the merge
   on `main` and pushes `v0.4.0`. The tag push runs CI first (as a
   `workflow_call`), then the release pipeline only if CI passes.
7. Prints the release run to follow:
   `To monitor the release job execute: gh run watch <id> -R geoffjay/tower`.

`--dry-run` stops after step 5's local commit and pushes nothing. If the
script is stopped while it waits for the merge, finish by hand after merging:
`git switch main && git pull && git tag v0.4.0 && git push origin v0.4.0`.

On a failed release run, re-run it with `workflow_dispatch` and the tag name;
the tag must already exist and point at the desired commit.

# Artifacts

Build matrix (native runners, `--locked`):

| Target | Runner |
|---|---|
| aarch64-apple-darwin | macos-latest |
| x86_64-apple-darwin | macos-15-intel |
| aarch64-unknown-linux-gnu | ubuntu-24.04-arm |
| x86_64-unknown-linux-gnu | ubuntu-latest |

Each target produces `tower-<target>.tar.gz` (binary + LICENSE files).
The release job merges them, generates `checksums.txt` (sha256), and
publishes a GitHub release via `softprops/action-gh-release` with
auto-generated notes; prereleases (`v*-*` tags) are marked as such.

# Versioning

Semantic versioning. While the major version is `0`, breaking changes are
expected between minor versions. The single source of truth is
`workspace.package.version`; `tower --version`, `/healthz`, and MCP
`serverInfo` all read `CARGO_PKG_VERSION`, so a release needs no code
change beyond the version bump.

# Homebrew tap

A `homebrew` job renders `packaging/homebrew/tower.rb.tpl` through
`scripts/gen-homebrew-formula.sh` (fills version + per-target sha256 from
the release's `checksums.txt`) and pushes `Formula/tower.rb` to
[geoffjay/homebrew-tap](https://github.com/geoffjay/homebrew-tap).

Requires the `HOMEBREW_TAP_TOKEN` repo secret (a PAT with write access to
the tap). If unset, the job logs a notice and skips — the GitHub release
still succeeds. Users install with `brew install geoffjay/tap/tower`.

# AUR (Arch Linux)

An `aur` job generates a `PKGBUILD` + `.SRCINFO` for `tower-bin` from
`packaging/aur/PKGBUILD.tpl` via `scripts/gen-aur-pkgbuild.sh` and pushes
them to `ssh://aur@aur.archlinux.org/tower-bin.git` using the `AUR_KEY`
repo secret (an SSH private key with access to the AUR package). If unset,
the job is skipped with a notice. Users install with `yay -S tower-bin`.

The PKGBUILD sources the release tarballs from GitHub (per-`$CARCH`
sha256), so the GitHub release must exist first — the job depends on the
release job. AUR's git server only accepts pushes to the `master` branch.

# Secrets

| Secret | Used by | Effect when unset |
|---|---|---|
| `GITHUB_TOKEN` | release job | required (built-in) |
| `HOMEBREW_TAP_TOKEN` | homebrew job | tap push skipped with notice |
| `AUR_KEY` | aur job | AUR push skipped with notice |

# Files involved

| File | Purpose |
|---|---|
| `scripts/release.sh` | `cargo make release -- --version X.Y.Z`: bump, gates, changelog, PR, tag after merge |
| `cliff.toml` | git-cliff config for `CHANGELOG.md` |
| `CHANGELOG.md` | Generated changelog (regenerated per release; not edited by hand) |
| `.github/workflows/release.yml` | Trigger (`v*` tag push, dispatch), CI gate, build matrix, release, tap, AUR |
| `.github/workflows/ci.yml` | Reused as the CI gate via `workflow_call` |
| `packaging/homebrew/tower.rb.tpl` | Formula template (never modified by hand per-release) |
| `scripts/gen-homebrew-formula.sh` | Renders the formula from checksums |
| `packaging/aur/PKGBUILD.tpl` | AUR package template |
| `scripts/gen-aur-pkgbuild.sh` | Renders PKGBUILD + `.SRCINFO` from checksums |
| `Cargo.toml` | `workspace.package.version` — the version source of truth |
