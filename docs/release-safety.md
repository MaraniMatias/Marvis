# Release and branch safety

## Integration snapshot (2026-10-04)

- This branch integrates audited fixes/schema 13 with main `5e68d0a` (`v0.13.2`, schema 11). All four version files have been restored to main's `0.13.2`; this integration is unreleased and still refuses schema-11/12 databases. The user reports the previous, unapproved `0.14.0` DMG exits at launch; its cause is not established and no app launch is verified. Any version bump is reserved for main after manual user testing and explicit approval. Checkpoint and integration commits are local; no tag, push, or GitHub settings change was made. Do not release from this worktree.
- Before 1.0, schema changes have no migration: schema 13 refuses schema 11/12 databases. Users must back up first and expect a new empty Marvis workspace; repository files are not deleted. Do not describe this as a data-preserving upgrade.
- Read-only GitHub API snapshot: `main` returned “Branch not protected” from the branch-protection endpoint and `[]` rulesets. The `main` commit check-runs were named `checks` and `linux-build` (both queued). This is a dated snapshot, not proof that checks passed or that protection remains absent.

## Required gates

The local `CI` workflow has two jobs; the `main` check-runs above confirm the exact check names to select in GitHub: **`checks`** and **`linux-build`** (not `CI / checks`). Recheck the PR's check-run names after the workflow changes merge, and require both before merge.

- `checks` (macOS): release-manifest check, formatting, lint, typecheck, full tests, security tests, coverage, app build, then mandatory `pnpm audit:security`. Coverage uploads with `if: always()` so failures still retain it.
- `linux-build` (Ubuntu 22.04): full tests (including the optimized vendored GLib iterator regression tests), Debian build, then `pnpm audit:rust`.
- `checks` must stay fail-closed. The upstream GLib 0.18.5 advisory (`RUSTSEC-2024-0429` / `GHSA-WRW7-89JP-8Q8G`) is considered fixed only when Cargo resolves the exact vendored backport and its complete source tree passes the pinned SHA-256 check. This is source verification, not an allowlist; every other finding, including new aliases or advisories, keeps the normal blocking policy. Never suppress or bypass a failed audit.
- Release validation reuses `CI`, then builds macOS arm64 and Linux x86_64/arm64 artifacts; publish waits for all builds and creates a published GitHub release. `release.yml` grants `contents: write` only to `publish`; preserve that scope and do not add broad write secrets.

## Owner/admin action, not performed

Applying this recommended mitigation is opt-in and requires owner approval plus an operator with repository-admin authority; this guide does not apply it, so S4 remains unresolved until then, and it must not disable or bypass required audits or checks.

After reviewing the integration and its data-loss impact, a repository owner/admin should configure `main` under **Settings → Rules → Rulesets** (or **Settings → Branches**): require a pull request and one approval, require status checks named exactly `checks` and `linux-build`, and configure no bypass actors/admin bypass. Use the names shown on the PR's check runs if GitHub reports different names after the workflow is merged. Do not weaken or skip the security audit to obtain a green merge.

After this integration is merged and all required checks pass, release only from a clean `main` checkout with explicit owner approval. `pnpm release <version>` is not read-only: it pushes the version commit and tag and publishes after CI. `--dry-run` on that command still pushes the version commit and dispatches validation; use `pnpm release:check` for read-only manifest validation. No release command is run as part of this checklist.

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
