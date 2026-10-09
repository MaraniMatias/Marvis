# Muster

Desktop app for running several coding agents in parallel on the same repo, one git worktree each, and keeping track of what every one of them is doing.

Each checkout gets its own window with its files, diffs, terminals and review notes. You write notes on specific diff lines and send them back to the agent, instead of pasting "the error is somewhere in that file" into a terminal.

**Platforms:** macOS (Apple Silicon) · Linux (amd64 / arm64)
**License:** MIT

---

## The problem

You hand a task to an agent and start working on another one. Then a second agent, then a third. Each gets its own worktree so nobody overwrites anybody. Git solves the isolation; nothing solves the bookkeeping:

- Which agent is working, which one stopped, which one is waiting for you?
- Which terminal belongs to which worktree?
- When an agent stops, you `cd` into its directory, start the app, try it, find something wrong, go back to the agent and describe it in prose. Or open the merge request in the browser, comment there, and copy the comments back.

Muster puts all of that in one window. Every checkout has its own group of terminals, its own diff and its own notes, and moving between them takes you to the right directory.

---

## What it does

- **One window per checkout.** Repo root or any worktree, each with its own files, diffs, terminals and notes. Nothing leaks between tabs.
- **Terminals grouped by checkout.** Several per checkout (agent, dev server, shell). Switching terminal switches working directory.
- **Worktree auto-detection.** Worktrees you create are picked up. So are the ones an agent creates.
- **Review notes on diff lines.** Draft → sent → resolved. When the code under a note changes, the note is marked outdated instead of pointing at something that is no longer there.
- **Two ways out for notes.** Send them straight to OpenCode, or export Markdown and paste it into any other agent.
- **Several repos at once.** Each repo is a group in the sidebar.
- **Diffs** side by side or unified, with files you can mark as viewed.
- **File tree and a simple editor** for quick fixes. Not meant for long editing sessions.

---

## Concepts

### Checkout

The unit Muster works with: the repository root, or any of its worktrees. The root is the checkout of whatever branch is currently on it, not necessarily `main`.

### Sidebar numbers

Next to each checkout you see lines added and removed.

- **Both, root and worktree:** everything the checkout has changed since it diverged from the repository's default branch, plus what is still uncommitted in the working tree. It answers "what did this agent do in total", not "what changed since I last looked". A checkout sitting on the default branch has no divergence of its own, so it shows only its uncommitted work.

### Review notes

A note is anchored to a line of a diff and goes through these states:

| State    | Meaning                                                                                   |
| -------- | ----------------------------------------------------------------------------------------- |
| Draft    | Written, not sent anywhere                                                                |
| Sent     | Delivered to an agent or exported                                                         |
| Resolved | Dealt with                                                                                |
| Outdated | The code under it changed; Muster flags it instead of leaving it claiming something false |

### Agents

Muster does not run or wrap an agent. You start it in a terminal inside the checkout.

- **OpenCode** has native integration: sessions, prompts, and review rounds with tracking so the same note is not sent twice ([test notes](docs/agent-live-tests.md)).
- **Anything else** (Claude Code, Codex, pi, Hermes, …): export the notes as Markdown and paste them in.

---

## What it is not

- Not an IDE.
- Not an LLM harness. It does not talk to any model.
- Not a GitHub or GitLab client.
- Not just a terminal.

Architecture and data flow: [docs/architecture.md](docs/architecture.md).

---

## Install

Download from the [latest release](https://github.com/MaraniMatias/Muster/releases/latest).

### macOS

```sh
# .dmg → drag to Applications
# First open may be blocked (ad-hoc signed):
xattr -dr com.apple.quarantine /Applications/Muster.app
```

### Debian / Ubuntu

```sh
sudo apt install ./Muster_<version>_amd64.deb   # or _arm64.deb
```

### Other Linux

```sh
chmod +x Muster_<version>_amd64.AppImage
./Muster_<version>_amd64.AppImage
```

Optional: install [OpenCode](https://opencode.ai) for the built-in agent integration.

---

## Quick start

1. **Open** a folder that is a git repo.
2. Create a worktree from the menu, or let an agent create one. It shows up in the sidebar either way.
3. Select a checkout in the sidebar.
4. Open a **terminal** in it and start your agent.
5. When the agent stops: run the app in another terminal of the same checkout and try it.
6. Open **Changes**, click a line in the diff, write a note. Repeat as needed.
7. Send the notes:
   - OpenCode → **Send to opencode**
   - Any other agent → export Markdown and paste it into its terminal

Then go back to your own task.

---

## Privacy

Everything stays on your machine. No telemetry.
What an agent sends to a model provider is up to that agent.

---

## Workspace data

Muster keeps repository registrations, review notes and rounds, and workspace state in `muster.sqlite3`, in the app-data directory for `dev.muster.workspace`. Repository and worktree files stay where they are. Terminal processes cannot outlive the app, so Muster clears terminal sessions and their layouts on startup; window size and UI state survive a restart.

Missing folders stay registered and are marked missing. If the path comes back, Muster restores that registration with its metadata.

If a directory was moved or renamed, the Sidebar offers no way to point the old registration at the new path. Opening the new path creates a separate checkout, and the two histories do not merge. Keep the old registration if you need its notes.

**Close missing** removes the registration and everything attached to it, notes and rounds included. The confirmation says no files are deleted, and that holds for repository and worktree files only.

Muster does not back up the database. Quit the app and make your own copy before confirming that action or deleting the file yourself.

---

## Troubleshooting

### Muster does not start

Check the database first. Muster does not migrate its schema. A build refuses a database stamped with a schema version it does not know, by name, rather than reshaping a file it did not write, and losing the repositories someone registered is worse than opening empty.

1. **Read the log.** It names the refusal.

   ```sh
   tail -n 60 "$HOME/Library/Logs/dev.muster.workspace/Muster.log"                        # macOS
   tail -n 60 "${XDG_DATA_HOME:-$HOME/.local/share}/dev.muster.workspace/logs/Muster.log" # Linux
   ```

2. **Ask the database which schema it holds.**

   ```sh
   DB="$HOME/Library/Application Support/dev.muster.workspace/muster.sqlite3"     # macOS
   DB="${XDG_DATA_HOME:-$HOME/.local/share}/dev.muster.workspace/muster.sqlite3"  # Linux
   sqlite3 "$DB" 'pragma user_version;'   # compare with SCHEMA_VERSION in src-tauri/src/persistence/mod.rs
   ```

3. **Reset it.** Quit Muster first: a running copy holds the file, and a second launch is turned away by the single-instance plugin.

   ```sh
   pkill -f -i muster                                # quit every Muster copy
   cp -v "$DB" "${DB%.sqlite3}.$(date +%s).sqlite3"   # keep a copy before removing it
   rm -v "$DB" "$DB"-wal "$DB"-shm "$DB"-journal
   ```

The next launch writes a fresh schema and opens an empty workspace.

This removes repository and checkout registrations, review notes and rounds, viewed files, per-checkout UI state, and the saved window size and position. It does not touch repository or worktree files, `~/.muster/config.yml` (UI, terminal and editor settings), or `~/.muster/tmp/code-reviews/` (review exports).

On Linux the log lives inside the database directory, so copy it out before removing anything.

### Coming from Marvis

The bundle identifier is now `dev.muster.workspace`, and everything derived from it moved with it. There is no migration: the app reads only the paths that carry its own name.

- **The old install stays installed.** `Marvis.app` and `Muster.app` are different applications with different identifiers, so macOS lists both and they do not replace each other. Delete `Marvis.app` by hand once you are happy with the new one.
- **The old app-data directory stays on disk.** `~/Library/Application Support/dev.marvis.workspace` and `~/Library/Logs/dev.marvis.workspace` (and their Linux equivalents under `$XDG_DATA_HOME`) are never read again, and nothing in them is deleted for you.
- **The database is new, not copied.** `muster.sqlite3` starts empty: no repository registrations, no review notes, no rounds. The old `marvis.sqlite3` is still a readable SQLite file if you want anything out of it, but the schema stamp is Marvis's and this build refuses a database it does not recognise.
- **Preferences move by hand.** Copy `~/.marvis/config.yml` to `~/.muster/config.yml` to keep your UI, terminal and editor settings; Muster writes the new file with its defaults on first launch, so copy over it rather than the other way round.
- **Review exports move by hand.** Markdown written by the export button lands in `~/.muster/tmp/code-reviews/` now. Anything under `~/.marvis/tmp/code-reviews/` is left where it was.

This is deliberate rather than unfinished: the project is `0.x`, so a rename that moves paths is made in place instead of being carried by a compatibility layer nobody asked for.

---

## Building

Source setup, build and artifact notes: [docs/development.md](docs/development.md).

Release and branch-gate checklist: [docs/release-safety.md](docs/release-safety.md).

---

## License

[MIT](LICENSE)
