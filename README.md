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
- Multiple terminals per checkout (splits, layouts saved)
- Optional OpenCode integration: sessions, prompts, send review rounds
- Works with any CLI agent (Claude Code, Codex, pi, …) or none

Not an IDE. Not a GitHub/GitLab client. Not a plain terminal.

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

---

## License

[MIT](LICENSE)
