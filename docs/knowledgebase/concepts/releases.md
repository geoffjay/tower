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
  by: omp/ollama-glm-5.3
  at: "2026-10-08T00:00:00Z"
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

1. Ensure `main` is clean, up to date, and green locally:
   ```
   cargo make lint && cargo make test
   ```
2. Bump `workspace.package.version` in the root `Cargo.toml` (all crates
   inherit it; `tower --version` reports it via `CARGO_PKG_VERSION`).
3. Tag and push:
   ```
   git tag v0.X.0
   git push origin v0.X.0
   ```
   The tag push runs CI first (as a `workflow_call`), then the release
   pipeline only if CI passes.
4. Monitor with `gh run watch`. On failure the workflow can be re-run via
   `workflow_dispatch` with the tag name; the tag must already exist and
   point at the desired commit.

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
| `.github/workflows/release.yml` | Trigger (`v*` tag push, dispatch), CI gate, build matrix, release, tap, AUR |
| `.github/workflows/ci.yml` | Reused as the CI gate via `workflow_call` |
| `packaging/homebrew/tower.rb.tpl` | Formula template (never modified by hand per-release) |
| `scripts/gen-homebrew-formula.sh` | Renders the formula from checksums |
| `packaging/aur/PKGBUILD.tpl` | AUR package template |
| `scripts/gen-aur-pkgbuild.sh` | Renders PKGBUILD + `.SRCINFO` from checksums |
| `Cargo.toml` | `workspace.package.version` — the version source of truth |
