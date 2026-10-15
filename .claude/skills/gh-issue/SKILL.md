---
name: gh-issue
description: Pick up and execute a GitHub issue from geoffjay/tower. Use when the user invokes /gh-issue with an issue number, a full GitHub address (https://github.com/geoffjay/tower/issues/N), or asks to work on "the next issue" or an issue by title from the repo issue list. Reads the issue, creates a work branch, implements the change with full repo workflow (design docs first, lint and test green), commits, opens a PR, and resets the local checkout to main.
argument-hint: "[issue number | URL | next | title fragment]"
---

# Execute a tower GitHub issue

End-to-end: issue → branch → change → commit → PR → back on main.
The user merges PRs and tags releases themselves. Never push to main.

## 0. Resolve the issue

Work from the tower repo root (`~/Projects/tower`).

```sh
gh issue list -R geoffjay/tower --state open
```

From the user's argument:

| Argument | Meaning |
|---|---|
| `N` (a number) | Issue `geoffjay/tower#N` |
| Full URL | Must be `github.com/geoffjay/tower/issues/N`; issues from other repos: ask |
| `next` or empty | Pick the lowest open number that has no open PR (check `gh pr list`) |
| Title fragment | The open issue whose title contains it; if several match, list them and ask |

```sh
gh issue view <N> -R geoffjay/tower --json number,title,body,labels,comments
```

Read the whole body and every comment before planning. Comments can change
the approach. If the issue references a design doc section (`D§N`), read
`docs/knowledgebase/concepts/design/<N>-*.md` too.

State the problem in your own words before starting. If the issue is a
question, not a change request, answer it in the issue comments and stop.

## 1. Branch

From an up-to-date main:

```sh
git checkout main && git pull
git switch -c issue/<N>-<short-slug>   # e.g. issue/12-service-path
```

Never commit work on main.

## 2. Implement

Follow the repo's own rules (AGENTS.md), in this order:

1. **Design doc first.** Any behavior change amends the matching
   `docs/knowledgebase/concepts/design/` section (cite the `D§` number)
   before code touches anything. Docs edits follow the okf skill
   (`okf fmt -w` and `okf validate` only; never `okf index` or
   `okf lint --fix`).
2. **Code.** Reuse existing patterns; no `unsafe`; no secrets.
3. **Gates.** `cargo make lint` and `cargo make test` must both be green.
   Update or add tests that cover the issue's actual failure mode — the
   issue body's "Verification sketch" or "Steps to reproduce" is the
   test plan. Existing snapshot tests: regenerate with
   `INSTA_UPDATE=always cargo test` and read the diff before accepting.
4. **Commits.** One commit per logical unit, conventional commits
   (`fix:`, `feat:`, `docs:`, `test:`, `chore:`, `spike:`). The pre-commit
   hook runs cargo-audit; if new RustSec advisories block the commit,
   `cargo update <crate>` to the fixed version and include the lockfile
   change. Docs and code land in separate commits.

## 3. Pull request

```sh
git push -u origin issue/<N>-<short-slug>
gh pr create --base main --title "<conventional-commit summary>" --body "..."
```

PR body sections: **Problem** (link `#N`, closes it), **Change** (what and
why, name the files/sections), **Verification** (what you actually ran —
commands and outcomes). Keep it short. The title should reference the
conventional-commit type of the main change.

If CI is required, watch `gh pr checks --watch` and fix failures on the
same branch. Post the PR link in the issue with `gh issue comment <N>
-R geoffjay/tower --body "PR: <url>"` if the issue will stay open past
this session.

## 4. Reset local

```sh
git checkout main
git branch -D issue/<N>-<short-slug>   # the branch lives on the remote in the PR
```

Do not merge the PR, tag a release, or bump the version — the user does
that. Say so if the fix should ship in the next release.

## Rules

- If any step turns out to conflict with what the issue or a maintainer
  comment says, follow the issue.
- If the change is unsafe, unsound, or much larger than the issue
  describes, stop and comment on the issue instead of improvising scope.
- Never force-push main, never push main directly.