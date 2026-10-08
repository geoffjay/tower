---
type: Plan
title: Release management
description: Tag-push release pipeline — CI gate, cross-platform tarballs, GitHub release, Homebrew tap, AUR package.
tags:
  - plan
  - releases
  - packaging
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

# Release management

Goal: pushing a `v*` tag produces a GitHub release with verified
cross-platform binaries, and updates the Homebrew tap and the AUR package.
Implements [Releases and packaging](../concepts/releases.md); distribution
model per [design §3](../concepts/design/03-process-and-deployment.md).

Precedents: nemo (Rust build matrix + formula template + tap push) and
horde (GoReleaser with AUR publish). tower is a Rust workspace, so the
pipeline is nemo-style; the AUR job follows horde's `-bin` pattern.
GoReleaser was rejected: single-runner darwin cross-compiles and a
deprecated `brews:` block; cargo-dist was rejected: no AUR support.

## Milestone 1 — Workflow

- **R1.1** `ci.yml`: add `workflow_call` trigger so the release pipeline
  can gate on CI. Verify: `gh workflow view` shows reusable; existing
  push/PR behavior unchanged.
- **R1.2** `.github/workflows/release.yml`: `v*` tag push +
  `workflow_dispatch(tag)`; `ci` job (uses ci.yml), `build` matrix job
  (4 native targets, `--locked`, tar.gz + LICENSE files), `release` job
  (checksums.txt + `softprops/action-gh-release`, prerelease on `-`,
  `fail_on_unmatched_files`), `homebrew` job (token gate → render →
  push `Formula/tower.rb` to `geoffjay/homebrew-tap`), `aur` job (key
  gate → render → push PKGBUILD + `.SRCINFO` to
  `aur@aur.archlinux.org:tower-bin.git`, branch `coordinator`).
  Verify: `actionlint` clean (or careful manual review); a `v0.0.0-test`
  dry-run can be inspected via workflow_dispatch without assets.

## Milestone 2 — Packaging assets

- **R2.1** `packaging/homebrew/tower.rb.tpl` + `scripts/gen-homebrew-formula.sh`:
  formula with `on_macos/on_arm|on_intel` + `on_linux` blocks sourcing
  the four tarballs, `test do` asserting `tower --version` output.
  Verify: script renders a formula from a synthetic checksums file;
  `brew audit --formula` locally (best effort, needs network).
- **R2.2** `packaging/aur/PKGBUILD.tpl` + `scripts/gen-aur-pkgbuild.sh`:
  PKGBUILD with per-`$CARCH` sha256 and `.SRCINFO` generation.
  Verify: script renders from a synthetic checksums file; `makepkg --printsrcinfo`
  round-trip on a Linux runner or manual inspection.

## Milestone 3 — Release

- **R3.1** Set `HOMEBREW_TAP_TOKEN` and `AUR_KEY` repo secrets
  (operator; both are PATs/SSH keys with push access to the tap and AUR
  package). Create the `tower-bin` AUR package repo first.
- **R3.2** Bump `workspace.package.version` to `0.1.0`, tag `v0.1.0`,
  push, monitor `gh run watch`. Verify: release page shows 4 tarballs +
  checksums.txt; `brew install geoffjay/tap/tower` and `tower --version`
  on macOS; `yay -S tower-bin` on Arch.

## Verification log

| Date | Check | Result |
|---|---|---|
