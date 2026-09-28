# Marvis

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux-lightgrey)

Marvis is a desktop app for working with git repositories and their checkouts. It puts a checkout's files, changes, terminals, and review notes in one window. You run whichever coding agent you prefer in a terminal inside the checkout, and Marvis can hand your review notes directly to OpenCode.

<!-- Add a screenshot or short GIF here, for example docs/screenshot.png -->

## What it solves

If you keep several worktrees of one repository open, files and terminals from different checkouts end up mixed across tabs and editor windows. You might be reviewing a pull request in one checkout while running commands in another, and nothing keeps them apart. In Marvis, each checkout has its own files, diffs, terminals, and agent, and you switch between checkouts from a sidebar.

Local review notes also go stale. You write "this is wrong", the file changes the next day, and the note ends up pointing at the wrong place. Marvis attaches each note to a specific line of a diff and checks whether that line still matches. Notes that no longer apply are marked as outdated instead of quietly misleading you.

And if you use a coding agent, giving it feedback usually means copying your comments into a separate chat. In Marvis, the agent runs in a terminal inside the same checkout, next to the diff. With OpenCode, Marvis sends your notes to it as one message. With any other agent, you export the notes as Markdown and paste them into its terminal.

Marvis is not an IDE, a GitHub or GitLab client, or a plain terminal. It is a working surface around a repository, and its editor is there for quick changes.

## What you can do with it

### Keep repositories and checkouts organized

Open a folder that contains a git repository and Marvis remembers it. A repository can have several checkouts (git worktrees) open at the same time, such as `main`, `feature/x`, and `fix/y`. The sidebar lists your repositories, their checkouts, and the terminals and sessions in each one, with the current state of each. When you restart Marvis, everything comes back as you left it.

You can create a new worktree from any checkout. When you delete one, Marvis first checks for uncommitted files, unmerged commits, and running sessions or agents, and asks you to confirm. If you moved a checkout's folder, you can point Marvis to the new location. If the folder is gone, you can close the checkout without touching the original repository.

### Browse and edit files

Each checkout has a file tree you can search by name. Files open in a simple code editor with syntax highlighting, so you can make quick changes without leaving Marvis. The editor is meant for small edits, not as a replacement for your main one. Marvis keeps a draft for every file, so switching files does not lose your changes. It only writes inside the checkout, and it will not overwrite read-only files.

Markdown files can be shown as source or as a rendered preview, with task lists shown as checkboxes and local images displayed inline.

### Read changes

Diffs open in their own view, side by side or unified, with the number of added and removed lines for each file and for the whole checkout. The Changes tab lists every modified file. You can mark files as viewed, and a counter shows how many you have left. Diffs update on their own when the repository changes, for example after you commit from a terminal.

### Review with notes

Click a line in a diff to write a note on it, or drag across several lines to cover a range. A note starts as a draft, becomes sent when it goes to the agent, and you mark it resolved once the change is made. When a file changes, Marvis re-checks each note and marks it outdated if its line moved or disappeared. "Clear outdated" removes those notes in one step.

Notes can be grouped into a review round. Marvis sends a round to the agent as a single message and never sends the same round twice, even if the app closes in the middle of sending. You can also export a round as Markdown and paste it anywhere.

### Run terminals

Every checkout can have as many terminals as you need, arranged in horizontal and vertical splits. Layouts are saved per checkout, and the sidebar shows which terminals are running and which have exited. Terminals are real shells, so you can run anything in them, including a coding agent such as Claude Code, Codex, OpenCode, or pi. They close when you quit Marvis.

### Work with a coding agent

Marvis does not tie you to one agent. Start Claude Code, Codex, OpenCode, pi, or any other command line agent in a terminal of the checkout, and it works on that checkout's files while your diffs update in the next panel.

OpenCode also has a built-in integration. If you use [OpenCode](https://opencode.ai), Marvis starts a separate local agent server for each checkout. It starts when the checkout needs it and stops when you close the checkout or the app. From Marvis you can create sessions, send prompts, stop a running turn, and pick from the models and agents you have available. When you send a review round, the agent is told to read each note and apply the change it describes.

The OpenCode integration is optional. Everything else works without it, with any agent or none.

### The window

Marvis has three panels: the sidebar on the left, the terminal, diff, or file in the middle, and an inspector with files and changes on the right. Panel sizes are remembered for each checkout, and in a narrow window the sidebar turns into a drawer. The menu is at the top of the window, in the macOS style. Errors and messages appear as small notifications that do not block your work. The theme is dark, and Marvis remembers where you left the window.

Only one copy of Marvis runs at a time. Opening it again brings the existing window to the front.

## Privacy

Marvis stores its data on your computer and sends no telemetry. What an agent sends to a model provider depends on that agent's own setup.

## Install

Marvis is an early release. Download the build for your system from the [latest release](https://github.com/MaraniMatias/Marvis/releases/latest).

### macOS

Download the `.dmg` and drag Marvis into Applications. The app is not code-signed, so macOS warns you the first time you open it. Right-click the app and choose Open, or run this once in a terminal:

```sh
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

### Debian and Ubuntu

Download the `.deb` and install it:

```sh
sudo apt install ./Marvis_<version>_amd64.deb
```

### Other Linux distributions

Download the `.AppImage`. It needs no installation:

```sh
chmod +x Marvis_<version>_amd64.AppImage
./Marvis_<version>_amd64.AppImage
```

### Optional: a coding agent

Marvis does not include one. Install the agent you like, such as Claude Code, Codex, or pi, the way you normally would, and run it in a Marvis terminal. To use the built-in OpenCode integration, install the [OpenCode](https://opencode.ai) CLI.

## Getting started

1. Open Marvis and use Open in the menu to choose a folder that contains a git repository.
2. Add a worktree from the menu if you want a second checkout of the same repository.
3. Pick a checkout in the sidebar. Browse files in the inspector, or open Changes to see what has been modified.
4. Open a diff and click a line to leave your first note.
5. Open a terminal in the checkout and run whatever you normally run there.
6. Start your agent in the terminal. With OpenCode, send your notes as a review round. With another agent, export them as Markdown and paste them in.

## License

[MIT](LICENSE)
