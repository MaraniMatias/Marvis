# MVP 0.3 decisions

Status after `git-spike/` and `terminal-spike/`. Scope is the user-mandated Vue 3 + TypeScript + Vite + shadcn-vue/Reka + Tailwind + Tauri 2 + Rust stack; Python is not in scope.

| Topic | Evidence and provisional direction | Status |
| --- | --- | --- |
| **Q1 — Worktree location** | Git spike confirms `<repo>/.worktrees/` appears as untracked in the primary checkout and explicitly does not choose a production location. Recommend configurable storage **outside** the repo: avoids untracked content and avoids editing `.git/info/exclude`; tradeoffs are separation from the repo and needing to manage path/name mapping. | **UNDECIDED — explicit user approval required before selecting a location or creating worktrees. No default path is chosen here.** |
| **Q11 — Uncertain default branch** | With `origin/HEAD`, the spike resolves the remote default branch. Without it, Git cannot reliably identify the original branch (including after switching branches); an unborn repo only supplies a candidate. Do not silently promote a candidate. Ask the user to confirm/select once and persist that choice per repo. | **Needs user confirmation of this fallback. Persistence was not implemented or tested by the spike.** |
| **Q7 — Submodules** | Observed: Git resolves an opened submodule as its own checkout and reports its superproject, but `worktree list` on Git 2.55.0 reports the submodule gitdir under `.git/modules/...`; the spike avoids exposing that gitdir, returns the opened submodule as its sole checkout, and warns. Linked worktrees from submodules remain unresolved. | **Provisional:** treat nested submodules as directories in the parent; if one is explicitly opened, handle it as a standalone checkout with a warning and do not claim linked-worktree support. Confirm before closing route resolution. |
| **Q8 — Bare repositories** | Observed: the spike rejects a bare repo with `Bare repository is not a checkout: <path>`. | **Provisional:** keep bare repos unsupported and return a clear error; confirm before closing route resolution. |

## Terminal gate

The terminal spike demonstrates headless PTY/backend behavior and builds/tests the isolated Tauri project, but its manual native-WebView checklist was not completed. Channel delivery/byte integrity, WebView responsiveness and rendering (including `nvim`/resize), and app/WebKit resource use remain unverified. **The end-to-end WebView gate is pending; do not declare the terminal spike passed until the checklist in `terminal-spike/README.md` is run and recorded.**

## Remaining decisions / checks

- Obtain explicit Q1 approval; until then, do not create production worktrees.
- Confirm Q11's ask-once-and-persist fallback and implement/test persistence.
- Confirm the provisional Q7/Q8 behavior before route resolution is considered closed.
- Complete and record the terminal spike's native WebView checklist.
