# Git spike findings (0.1)

This is a runnable Rust experiment only; it does not choose a production worktree location or implement the app's Git service. Tests and benchmarks create disposable repositories under the OS temporary directory. The `.worktrees/` case is only a fixture. The isolated crate is in `git-spike/`.

## Run

```sh
cargo test --manifest-path git-spike/Cargo.toml
cargo run --manifest-path git-spike/Cargo.toml -- resolve /path/inside/a/checkout
cargo run --manifest-path git-spike/Cargo.toml -- status /path/to/checkout
cargo run --release --manifest-path git-spike/Cargo.toml -- bench --counts 100 5000 50000 --repeats 5
```

All Git invocations in the spike use argument arrays through Rust's `Command`, not a shell command string. Resolution uses `rev-parse`, `symbolic-ref`, and `git worktree list --porcelain`; it canonicalizes existing input paths and identifies the containing checkout from Git's top-level result. The status command is `git status --porcelain=v2`.

## Resolution observations

| Case | Expected / observed |
| --- | --- |
| Primary checkout; primary opened through nested directory or symlink | Resolves to the same canonical primary checkout. |
| Worktree opened without first opening its primary; nested directory inside worktree | Git identifies the primary repo, the opened worktree, and the full linked-worktree list. The worktree checkout is not mistaken for a second repo. |
| Worktree below `<repo>/.worktrees/` | Resolves normally. In this fixture, the primary checkout's status reports `.worktrees/` as untracked. This is an observation, not a location recommendation; Q1 remains open. |
| Missing worktree directory | `git worktree list --porcelain` retains it and marks it `prunable`; the spike marks it `exists: false`. Opening the missing path itself fails clearly because there is no path to canonicalize. Git stops listing it after pruning. |
| Detached HEAD | Branch is `null`; commit HEAD remains available, and Git marks that worktree detached. |
| Unborn repository with `git init -b trunk` | Branch is `trunk`; HEAD is `null`. The default branch is not verifiable from `origin/HEAD`, so the result is a candidate requiring confirmation. |
| No remote, non-`main` branch | Git exposes the current symbolic branch but does not record which branch was the repository's original branch. After switching from `trunk` to `feature`, only `feature` is inferable. The spike marks the current branch as a candidate requiring confirmation; the app must request and persist a choice rather than silently treating it as default. Persistence is outside this spike. |
| Remote with `refs/remotes/origin/HEAD` | `git symbolic-ref --short refs/remotes/origin/HEAD` resolves `origin/main`; the spike returns `main` as authoritative. |
| Branch already used by another checkout | Git rejects a second `worktree add` for that branch. The worktree list maps the branch to the checkout path, which can be used for a Marvis-specific actionable message. |
| Submodule | `rev-parse --show-toplevel` resolves the submodule checkout and `--show-superproject-working-tree` identifies its parent. On Git 2.55.0, `git worktree list --porcelain` reports the submodule's gitdir under `.git/modules/...` rather than its working directory. The spike avoids presenting that gitdir as a checkout, returns the opened submodule as its sole checkout, and attaches a warning. Whether to support linked worktrees created from submodules remains unresolved. |
| Bare repository | Rejected with `Bare repository is not a checkout: <path>` instead of being represented as a checkout. |

Canonical paths are suitable for checkout identity in the tested primary/worktree/subdirectory/symlink cases. Missing paths cannot be canonicalized from the path alone; reconcile these from a surviving checkout's worktree listing.

## Status timing

Environment: macOS 26.6.2, arm64, Git 2.55.0, Rust 1.98.1. Temporary repositories contained tracked files with identical small contents. Each sample measures the complete Git process invocation, including startup; five samples were run per size, and the median is shown.

| Fixture size | Samples (ms) | Median (ms) | v0.4 budget | Result |
| ---: | --- | ---: | ---: | --- |
| 100 (<5k) | 9.09, 8.99, 8.78, 8.73, 10.88 | 8.99 | <100 ms | Pass |
| 5,000 (5k–50k) | 72.51, 76.63, 75.76, 74.91, 74.82 | 74.91 | <300 ms | Pass |
| 50,000 (50k–200k) | 84.22, 84.26, 85.64, 85.74, 86.27 | 85.64 | <1 s | Pass |

These are local fixture timings, not a general performance guarantee. They cover tracked files only, one machine/filesystem, and no concurrent filesystem churn. The app must still run status asynchronously/off the UI thread; this spike does not measure UI responsiveness.

## Test coverage and boundaries

The Rust test harness has eight tests covering the path, branch, missing-worktree, branch-in-use, submodule, bare, and status cases above. Tests create only temporary fixtures and leave the repository's existing files alone. The benchmark also builds and removes temporary fixtures.

The experiment does not persist a user-selected default branch, prune missing entries, create production worktrees, resolve plain folders, or decide Q1. The submodule worktree-list discrepancy and reliable identification of a no-remote repository's original branch need an explicit product decision/handling before treating those cases as fully resolved.
