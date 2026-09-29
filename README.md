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

You can create a new worktree from any checkout. A worktree you are done with can be archived instead of deleted: it leaves the sidebar and its files stay exactly where they are, and the repo row brings back everything you archived from it in one click. When you delete one, Marvis first checks for uncommitted files, unmerged commits, and running sessions or agents, and asks you to confirm. If you moved a checkout's folder, you can point Marvis to the new location. If the folder is gone, you can close the checkout without touching the original repository.

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

Release tags are `v<version>` and must match `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. Builds target Apple Silicon (`aarch64-apple-darwin`), x86_64 Linux (`amd64`) and arm64 Linux (`arm64`); Intel macOS is not currently published. Every release also carries a `SHA256SUMS` file, so a downloaded build can be checked with `sha256sum -c SHA256SUMS`.

### Releasing

One command ships a release. It checks the repository can take one, writes the version into all three manifests, commits, pushes the commit and the tag, waits for the build, and publishes the release:

```sh
pnpm release v0.2.0                     # the whole thing
pnpm release v0.2.0 --dry-run           # bump and build, no tag
pnpm release:preview v0.2.0             # report the writes, change nothing
RELEASE_SKIP_PUBLISH=1 pnpm release v0.2.0   # stop at the draft
```

Every step is skipped when the state already satisfies it, so `--dry-run` followed by the plain command is the two-command version of the same release: the first leaves the version committed on `main` and builds it, the second finds that work done and only has the tag to push. The same command resumes a release whose build failed, or a draft that was never published, and it refuses to touch a release that is already public.

It refuses to run off `main`, on a working tree with uncommitted changes, without `gh auth login`, or on a tag that does not point at the version. `pnpm release:check` runs on every pull request and reports any drift between the manifests.

Do not assume this workflow blocks old release tags: GitHub runs the workflow YAML stored at the tagged commit. For example, `v0.1.0` points to `57cda5e`, whose older workflow publishes directly on a tag push without the current checks. This YAML cannot block that workflow. Until the remote tag and release restrictions below have been configured and verified, releases are **unsafe and not ready**: do not create, push, or retag `v*` tags, and do not create or publish releases manually. Once those controls are verified, `pnpm release <version> --dry-run` runs the full checks and builds on `main` without creating the tag.

The tag is pushed before the build is known to pass, so the release lands as a **draft** first and is published only once every matrix job succeeded. A failed build therefore leaves a tag with no release at all rather than a half-built one, and `gh run rerun <id>` retries the same tag without a new version.

Before enabling releases, an administrator must protect `main` with pull-request and required-check rules; create an active `v*` tag ruleset that restricts tag creation, updates, and deletions, with bypass limited to named release maintainers; and restrict who may create, edit, or delete releases. Set and verify the repository's publication policy (including immutable releases where available). These remote settings are prerequisites, not enforced by this workflow. The API check immediately before publishing detects tag movement but cannot close the race with a later update; immutable-tag protection is required.

Release readiness must be verified against GitHub while authenticated: run `gh auth status`, inspect `gh api 'repos/MaraniMatias/Marvis/rulesets?includes_parents=true'` and `gh api repos/MaraniMatias/Marvis/branches/main/protection`, confirm the tag ruleset and release immutability/settings in repository settings, then resolve the remote tag:

```sh
TAG=v1.2.3
gh api "repos/MaraniMatias/Marvis/git/ref/tags/$TAG"
```

For an annotated tag, follow each tag object's `object.sha` with `gh api "repos/MaraniMatias/Marvis/git/tags/$OBJECT_SHA"` (set `OBJECT_SHA` to the returned SHA) until the target is a commit. Compare that commit SHA with the successful dry-run/workflow SHA. If authentication or permission is unavailable, record the remote protections as **unverified** and do not declare release readiness.

### macOS

Download the `.dmg` and drag Marvis into Applications. The app is ad-hoc signed, not signed with an Apple Developer ID, so macOS still warns you the first time you open it. Right-click the app and choose Open, or run this once in a terminal:

```sh
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

### Debian and Ubuntu

Download the `.deb` matching your architecture and install it:

```sh
sudo apt install ./Marvis_<version>_amd64.deb   # x86_64
sudo apt install ./Marvis_<version>_arm64.deb   # arm64
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
