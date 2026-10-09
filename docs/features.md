# Muster feature catalog

This catalog describes the implemented behavior in Muster v0.20.0. It separates user-facing features from backend-only commands and planned integrations; a registered command or a design document is not, by itself, a shipped UI feature.

## At a glance

Muster is a local desktop workspace for repositories and worktrees. It brings file browsing and editing, Git changes, line-anchored review notes, OpenCode sessions, and per-checkout terminals into one resizable window.

## Implemented features

### Workspace and worktrees

The sidebar groups registered repositories and their checkouts. Selecting a checkout makes it active across the files, changes, document, and terminal areas. The repository menu can create a worktree with a chosen branch and location. Removing a worktree requires confirmation.

Sources: [App shell](../src/App.vue), [Sidebar](../src/components/Sidebar.vue), [Worktree dialog](../src/components/WorktreeDialog.vue), [workspace commands](../src-tauri/src/commands/workspace.rs), [worktree service](../src-tauri/src/services/worktree.rs).

### Browse, edit, and preview files

The Files tab navigates the active checkout. Source files open in the document pane, where they can be edited and saved. Language detection and syntax highlighting are available for source documents, with a manual language selector. Sticky folder rows are configurable.

Markdown files can be shown as rendered pages with tables, task lists, fenced-code highlighting, front-matter metadata, and checkout-relative images. Raw HTML is disabled, output is sanitized, and image paths are restricted to the checkout; remote images are not fetched for the preview.

Sources: [Inspector pane](../src/components/InspectorPane.vue), [Document pane](../src/components/DocumentPane.vue), [code editor](../src/lib/code-editor.ts), [language catalog](../src/lib/source-languages.ts), [Markdown renderer](../src/lib/markdown-preview.ts), [preview presentation](../src/presentation/markdown-preview.ts).

### Git changes and diffs

The Changes tab lists changed files with added and removed line counts. Open one file's diff or the change set together, and review updates as repository changes are detected.

Sources: [Inspector pane](../src/components/InspectorPane.vue), [FileDiff](../src/components/FileDiff.vue), [active Git snapshot](../src/presentation/active-git-snapshot.ts), [Git watcher](../src/presentation/git-watchers.ts), [Git service](../src-tauri/src/services/git.rs).

### Review notes and delivery

Notes attach to lines in a diff. Muster tracks sent, resolved, and outdated notes; outdated anchors can be accepted, and notes can be edited or deleted. Open notes can be bundled into a review round, directed to a selected OpenCode session, and sent or queued. Markdown export is also available for use outside OpenCode. The review UI reports unfinished rounds and can include outdated notes.

Sources: [FileDiff](../src/components/FileDiff.vue), [ReviewNoteList](../src/components/ReviewNoteList.vue), [ReviewComposer](../src/components/ReviewComposer.vue), [review-note presentation](../src/presentation/review-notes.ts), [review-round service](../src-tauri/src/services/review_round.rs).

### OpenCode sessions

Muster connects to an OpenCode API v2 service that is already running locally. It does not launch OpenCode for you. Sessions and activity appear alongside the workspace; the association between a session and a terminal is inferred from terminal titles, rather than from an explicit shell session identifier.

Sources: [Sidebar](../src/components/Sidebar.vue), [session presentation](../src/presentation/agent-sessions.ts), [OpenCode client](../src-tauri/src/services/opencode.rs), [agent bridge](../src-tauri/src/services/agent.rs).

### Integrated terminals

Open multiple shell terminals in a checkout and interact with them in the workspace. Terminal settings include font size, ligatures, blinking and cursor shape, scrollbar visibility, shell integration, and worktree-following behavior. Following can be disabled or triggered by shell navigation, OpenCode activity, or either.

Terminals are runtime processes: Muster does not restore them as running sessions after an application restart.

Sources: [TerminalSession](../src/components/TerminalSession.vue), [terminal setup](../src/lib/muster-terminal.ts), [terminal service](../src-tauri/src/services/terminal.rs), [persistence startup behavior](../src-tauri/src/persistence/mod.rs).

### Layout and preferences

The navigation, document, and inspector areas can be resized, and the workspace offers split and focused layout modes. Muster remembers workspace and checkout UI state. Preferences are stored separately in ~/.muster/config.yml.

| Area | Default | Available settings |
| --- | --- | --- |
| Interface | 14 px, 100% scale, system theme, #16181c content background, sticky folders on | Font size 11–20 px; scale 80–150% in 10-point steps; system/light/dark theme; content background; sticky folder rows |
| Terminal | 16 px; ligatures and blinking on; block cursor; hidden scrollbar; shell integration on; follows agent activity | Font size 9–32 px; ligatures; cursor blink and block/bar/underline shape; hidden/automatic/always scrollbar; shell integration; follow off, cd, agent, or both |
| Editor | 13 px; ligatures and blinking on; spaces, 2 columns | Font size 9–32 px; ligatures; cursor blink; tabs or spaces; indentation 1–8 columns |

Sources: [App shell](../src/App.vue), [UI state](../src/domain/ui-state.ts), [layout persistence](../src/presentation/layout-persistence.ts), [settings model and defaults](../src/domain/settings.ts), [zoom options](../src/domain/zoom.ts).

## Local data and startup

Workspace, checkout UI, and review data are stored locally; settings live in the local configuration file. Muster restores the workspace but does not restart terminal processes. The SQLite schema uses a strict version check and has no migration ladder: a database stamped with an unrecognized schema version is refused rather than upgraded.

Sources: [Persistence](../src-tauri/src/persistence/mod.rs), [settings config](../src-tauri/src/config.rs).

## Platform packages

The configured release targets are macOS arm64 (Apple Silicon) as a DMG, and Linux amd64/arm64 as DEB and AppImage packages. Windows is not a configured release target.

Sources: [Tauri bundle config](../src-tauri/tauri.conf.json), [release workflow](../.github/workflows/release.yml).

## Backend-only commands

These exist in the backend but are not exposed as current UI actions. They should not be promoted to user-facing feature claims without a UI entry point:

- git_viewed_files and git_mark_viewed.
- locate_missing_checkout; do not claim that the UI automatically finds or reattaches a moved checkout.
- Direct agent_prompt and agent_stop; review delivery uses the review-round flow instead.

Sources: [registered commands](../src-tauri/src/main.rs), [Git commands](../src-tauri/src/commands/git.rs), [workspace commands](../src-tauri/src/commands/workspace.rs), [agent commands](../src-tauri/src/commands/agent.rs).

## Specified, not implemented

The following review destinations are planned in design documents, but are not available in the shipped UI:

- [Codex](todo/phase-2-codex.md)
- [Claude Code](todo/phase-3-claude-code.md)
- [pi](todo/phase-4-pi.md)

## Catalog files

[features.json](features.json) is the machine-readable counterpart to this document. It contains no screenshot selectors, fixture assumptions, or capture storyboard. Source tests were inspected during the audit but are not claimed as executed.
