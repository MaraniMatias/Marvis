# Release and branch safety

## Candidate snapshot (2026-10-04)

- This checkout is a dirty candidate at `0.11.0` / schema 13; the prior version is `0.10.0` / schema 11. No release, commit, tag, push, or GitHub settings change was made. Do not release from this worktree.
- Before 1.0, schema changes have no migration: schema 13 refuses schema 11/12 databases. Users must back up first and expect a new empty Marvis workspace; repository files are not deleted. Do not describe this as a data-preserving upgrade.
- Read-only GitHub API snapshot: `main` returned “Branch not protected” from the branch-protection endpoint and `[]` rulesets. The `main` commit check-runs were named `checks` and `linux-build` (both queued). This is a dated snapshot, not proof that checks passed or that protection remains absent.

## Required gates

The local `CI` workflow has two jobs; the `main` check-runs above confirm the exact check names to select in GitHub: **`checks`** and **`linux-build`** (not `CI / checks`). Recheck the PR's check-run names after the workflow changes merge, and require both before merge.

- `checks` (macOS): release-manifest check, formatting, lint, typecheck, full tests, security tests, coverage, app build, then mandatory `pnpm audit:security`. Coverage uploads with `if: always()` so failures still retain it.
- `linux-build` (Ubuntu 22.04): full tests, Debian build, then `pnpm audit:rust`.
- `checks` must stay fail-closed. The workflow documents the current moderate upstream GLib 0.18.5 advisory (`RUSTSEC-2024-0429` / `GHSA-WRW7-89JP-8Q8G`) as an expected blocker. Resolve it and get green required checks before release; do not suppress it, add an allowlist, or bypass the failed job. Any proposed risk acceptance needs explicit repository-owner approval recorded in the PR; it does not make a failed required check pass.
- Release validation reuses `CI`, then builds macOS arm64 and Linux x86_64/arm64 artifacts; publish waits for all builds and creates a draft. `release.yml` grants `contents: write` only to `publish`; preserve that scope and do not add broad write secrets.

## Owner/admin action — not performed

Applying this recommended mitigation is opt-in and requires owner approval plus an operator with repository-admin authority; this guide does not apply it, so S4 remains unresolved until then, and it must not disable or bypass required audits or checks.

After reviewing this candidate and its data-loss impact, a repository owner/admin should configure `main` under **Settings → Rules → Rulesets** (or **Settings → Branches**): require a pull request and one approval, require status checks named exactly `checks` and `linux-build`, and configure no bypass actors/admin bypass. Use the names shown on the PR's check runs if GitHub reports different names after the workflow is merged. Do not weaken or skip the security audit to obtain a green merge.

After the candidate is merged and all required checks pass, release only from a clean `main` checkout with explicit owner approval. `pnpm release <version>` is not read-only: it pushes the version commit and tag and publishes after CI. `--dry-run` on that command still pushes the version commit and dispatches validation; use `pnpm release:check` for read-only manifest validation. No release command is run as part of this checklist.

## Read-only verification

```sh
git status --short --branch
pnpm release:check
gh api repos/MaraniMatias/Marvis/branches/main/protection
gh api repos/MaraniMatias/Marvis/rulesets
gh api repos/MaraniMatias/Marvis/commits/main/check-runs --jq '.check_runs[] | [.name,.status,.conclusion] | @tsv'
gh pr checks <PR-number> --repo MaraniMatias/Marvis
```

The branch-protection endpoint returns 404 when no classic rule protects `main`; inspect rulesets separately. These GET/check commands do not change remote state.
