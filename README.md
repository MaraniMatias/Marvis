# Marvis

Desktop app for working with git repos and their worktrees.

Keeps files, diffs, terminals, and review notes in one window per checkout.
Run any coding agent in a terminal inside that checkout. Send review notes to OpenCode (or export as Markdown).

**Platforms:** macOS (Apple Silicon) · Linux (amd64 / arm64)  
**License:** MIT

---

## Why

Multiple worktrees → files and terminals get mixed across tabs and windows.

Review notes go stale when the code moves.

Copying feedback to an agent is tedious.

Marvis keeps each checkout isolated and attaches notes to specific diff lines. Notes that no longer match are marked outdated.

---

## What it does

- Open a repo → Marvis remembers it and its worktrees
- Multiple checkouts side by side (`main`, `feature/x`, …)
- File tree + simple editor (for quick edits)
- Diffs (side-by-side or unified), mark files as viewed
- Notes on diff lines → draft → sent → resolved. Auto-detects outdated ones
- Multiple terminals per checkout (splits/layouts last for the current run; reset on restart)
- Optional OpenCode integration: sessions, prompts, send review rounds ([test notes](docs/agent-live-tests.md))
- Works with any CLI agent (Claude Code, Codex, pi, …) or none

Not an IDE. Not a GitHub/GitLab client. Not a plain terminal.

Architecture and data flow: [docs/architecture.md](docs/architecture.md).

---

## Install

Download from the [latest release](https://github.com/MaraniMatias/Marvis/releases/latest).

### macOS

```sh
# .dmg → drag to Applications
# First open may be blocked (ad-hoc signed):
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

### Debian / Ubuntu

```sh
sudo apt install ./Marvis_<version>_amd64.deb   # or _arm64.deb
```

### Other Linux

```sh
chmod +x Marvis_<version>_amd64.AppImage
./Marvis_<version>_amd64.AppImage
```

Optional: install [OpenCode](https://opencode.ai) if you want the built-in agent integration.

---

## Quick start

1. **Open** a folder that is a git repo
2. (Optional) **Add worktree** from the menu
3. Select a checkout in the sidebar
4. Open **Changes** → click a line in a diff → write a note
5. Open a **terminal** in that checkout and run your agent
6. Send notes:
   - OpenCode → “send review round”
   - Other agent → export Markdown and paste

---

## Privacy

Everything stays on your machine. No telemetry.
What an agent sends to a model provider is up to that agent.

## Workspace data

Marvis stores repository registrations, review notes and rounds, and workspace state in `marvis.sqlite3` under the OS-specific app-data directory for `dev.marvis.workspace`. Repository and worktree files are separate. Terminal processes cannot survive app exit, so startup clears terminal sessions and their layouts; window geometry and UI state persist.

Missing folders remain registered and are marked missing. If the original path returns, Marvis restores that registration and its attached metadata. If a directory was moved or renamed, the Sidebar offers no relocation action: opening the new path creates a separate checkout identity and does not merge the missing checkout's history. Keep the old missing registration if you need that history. **Close missing** removes the Marvis registration and associated metadata (including notes and rounds); its confirmation that no files are deleted refers to repository/worktree files. Marvis does not back up the database for you: quit the app and make a separate backup before confirming that action or manually deleting the database.

This `v0.11.0` build expects database schema 13 and refuses older nonzero schemas, including 11 and 12. Pre-1.0 releases do not migrate older schemas: deleting the database starts an empty Marvis workspace, without deleting repository files or automatically recovering workspace metadata. Details and source setup: [docs/development.md](docs/development.md).

## Building

Source setup: [docs/development.md](docs/development.md). Build and artifact notes: [docs/build.md](docs/build.md).

Release and branch-gate checklist: [docs/release-safety.md](docs/release-safety.md).

---

## License

[MIT](LICENSE)
