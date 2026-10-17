#!/usr/bin/env bash
# Cut a release (docs/knowledgebase/concepts/releases.md).
#
#   cargo make release -- --version 1.2.3 [--dry-run]
#
# Bumps workspace.package.version, runs the gates, regenerates CHANGELOG.md
# with git-cliff, commits on release/v1.2.3, opens a PR and watches its
# checks, waits for the PR to be merged, then tags the merge on main and
# pushes v1.2.3 — which starts the Release workflow. --dry-run stops after
# the local commit (nothing is pushed).
set -euo pipefail

usage() {
  echo "usage: cargo make release -- --version X.Y.Z [--dry-run]" >&2
  exit 2
}
die() { echo "release: $*" >&2; exit 1; }
step() { echo; echo "== $* =="; }

VERSION=""
DRY_RUN=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) [[ $# -ge 2 ]] || usage; VERSION="${2#v}"; shift 2 ;;
    --version=*) VERSION="${1#--version=}"; VERSION="${VERSION#v}"; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    --) shift ;; # cargo make forwards the separator itself
    -h|--help) usage ;;
    *) echo "release: unknown argument: $1" >&2; usage ;;
  esac
done
[[ -n "$VERSION" ]] || usage
SEMVER='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
[[ "$VERSION" =~ $SEMVER ]] || die "'$VERSION' is not a semver version (X.Y.Z or X.Y.Z-pre)"
TAG="v$VERSION"
BRANCH="release/$TAG"

cd "$(git rev-parse --show-toplevel)"

step "preflight"
for tool in git gh git-cliff cargo; do
  command -v "$tool" >/dev/null || die "$tool not found (git-cliff: cargo install git-cliff)"
done
[[ -z "$(git status --porcelain)" ]] || die "working tree is not clean"
# run from main: bash reads this file lazily, so never switch branches under
# it before the release branch is cut from the same content
[[ "$(git branch --show-current)" == "main" ]] || die "run from main (git switch main)"
git pull --quiet --ff-only
git fetch --quiet --tags
git rev-parse --quiet --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists"
git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1 &&
  die "branch $BRANCH already exists on origin"
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
# the workspace version is the only top-level `version =` line in Cargo.toml
[[ $(grep -c '^version = ' Cargo.toml) -eq 1 ]] ||
  die "expected exactly one top-level 'version =' line in Cargo.toml"
CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
[[ "$CURRENT" != "$VERSION" ]] || die "Cargo.toml is already at $VERSION"
echo "$REPO: $CURRENT → $VERSION ($TAG)"

step "bump workspace version"
git switch --quiet -c "$BRANCH"
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml && rm Cargo.toml.bak

step "gates: cargo make lint"
cargo make lint
step "gates: cargo make test"
cargo make test
REPORTED=$(cargo run --quiet -p tower-server --bin tower -- --version)
[[ "$REPORTED" == "tower $VERSION" ]] || die "tower --version reports '$REPORTED'"
echo "$REPORTED"

step "changelog (git-cliff)"
git-cliff --tag "$TAG" -o CHANGELOG.md
NOTES=$(git-cliff --tag "$TAG" --unreleased --strip all)

step "commit"
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit --quiet -m "chore: release $TAG"
git show --stat --oneline HEAD | cat

if $DRY_RUN; then
  echo
  echo "Dry run: stopped before push. Release notes:"
  echo "$NOTES"
  echo
  echo "Discard with: git switch main && git branch -D $BRANCH"
  exit 0
fi

step "pull request"
git push --quiet -u origin "$BRANCH"
PR_URL=$(gh pr create --repo "$REPO" --base main --head "$BRANCH" \
  --title "chore: release $TAG" \
  --body "$(printf 'Release %s: version bump + CHANGELOG.md.\n\n%s\n' "$TAG" "$NOTES")")
echo "$PR_URL"

step "watch PR checks"
# checks take a few seconds to register on a fresh PR
for _ in $(seq 1 30); do
  gh pr checks "$PR_URL" 2>&1 | grep -q "no checks reported" || break
  sleep 5
done
gh pr checks "$PR_URL" --watch

step "wait for merge"
echo "Merge $PR_URL to continue (polling every 15s)."
echo "If you stop here, after merging run:"
echo "  git switch main && git pull && git tag $TAG && git push origin $TAG"
while true; do
  STATE=$(gh pr view "$PR_URL" --json state -q .state)
  case "$STATE" in
    MERGED) break ;;
    CLOSED) die "$PR_URL was closed without merging" ;;
  esac
  sleep 15
done

step "tag $TAG"
git switch --quiet main
git pull --quiet --ff-only
MAIN_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
[[ "$MAIN_VERSION" == "$VERSION" ]] || die "main is at $MAIN_VERSION after the merge, not $VERSION"
git tag "$TAG"
git push --quiet origin "$TAG"
git branch --quiet -D "$BRANCH" 2>/dev/null || true

step "release workflow"
RUN_ID=""
for _ in $(seq 1 12); do
  RUN_ID=$(gh run list --repo "$REPO" --workflow release.yml --branch "$TAG" \
    --limit 1 --json databaseId -q '.[0].databaseId // empty')
  [[ -n "$RUN_ID" ]] && break
  sleep 5
done
if [[ -n "$RUN_ID" ]]; then
  echo "To monitor the release job execute: gh run watch $RUN_ID -R $REPO"
else
  echo "Release run not listed yet; find it with: gh run list -R $REPO --workflow release.yml"
fi
