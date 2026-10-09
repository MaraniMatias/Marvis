# Muster feature catalog

This catalog describes the implemented behavior in Muster v0.20.0. It separates user-facing features from backend-only commands and planned integrations; a registered command or a design document is not, by itself, a shipped UI feature.

## At a glance

Muster is a local desktop workspace for repositories and worktrees. It brings file browsing and editing, Git changes, line-anchored review notes, OpenCode sessions, and per-checkout terminals into one resizable window.

## Implemented features

### Workspace and worktrees

The sidebar groups registered Git repositories and plain folders with their checkouts. Open directory registers a selected folder; the titlebar workdir menu also offers recently opened paths. Selecting a checkout makes it active across the files, changes, document, and terminal areas. Worktree creation lets you name a task and new branch, but always starts from main and uses the repository's .worktrees directory. A worktree can be archived without changing disk or deleted with explicit safeguards.

Sources: [App shell](../src/App.vue), [Sidebar](../src/components/Sidebar.vue), [Worktree dialog](../src/components/WorktreeDialog.vue), [workspace commands](../src-tauri/src/commands/workspace.rs), [worktree service](../src-tauri/src/services/worktree.rs).

### Browse, edit, and preview files

The Files tab navigates the active checkout. Source files open in the document pane, where they can be edited and saved. Language detection and syntax highlighting are available for recognized source files, with a searchable manual language selector. The CodeMirror editor includes line numbers, undo history, folding, bracket matching/closing, rectangular and multiple selections, configured indentation, and changed-line markers. Sticky folder rows are configurable.

Markdown files can be shown as rendered pages with tables, task lists, fenced-code highlighting, front-matter metadata, and checkout-relative images. Dedicated previews also support PNG, JPG/JPEG, GIF, WebP, AVIF, ICO, BMP, MP4, WebM, MOV, OGV, and SVG. Raw HTML is disabled, output is sanitized, and image paths are restricted to the checkout; remote images are not fetched for the preview.

Sources: [Inspector pane](../src/components/InspectorPane.vue), [Document pane](../src/components/DocumentPane.vue), [code editor](../src/lib/code-editor.ts), [language catalog](../src/lib/source-languages.ts), [Markdown renderer](../src/lib/markdown-preview.ts), [preview presentation](../src/presentation/markdown-preview.ts).

### Git changes and diffs

The Changes tab lists changed files with added and removed line counts. Open one file's diff or the change set together, and review updates as repository changes are detected.

Sources: [Inspector pane](../src/components/InspectorPane.vue), [FileDiff](../src/components/FileDiff.vue), [active Git snapshot](../src/presentation/active-git-snapshot.ts), [Git watcher](../src/presentation/git-watchers.ts), [Git service](../src-tauri/src/services/git.rs).

### Review notes and delivery

Notes attach to lines or ranges in a diff and progress through draft, sent, and resolved states. Outdated is a separate flag for changed anchors; outdated anchors can be accepted, and notes can be edited or deleted. Open notes can be bundled into a review round, directed to a selected OpenCode session, and sent or queued. Markdown export is also available for use outside OpenCode. The review UI reports unfinished rounds and can include outdated notes.

Sources: [FileDiff](../src/components/FileDiff.vue), [ReviewNoteList](../src/components/ReviewNoteList.vue), [ReviewComposer](../src/components/ReviewComposer.vue), [review-note presentation](../src/presentation/review-notes.ts), [review-round service](../src-tauri/src/services/review_round.rs).

### OpenCode sessions

Muster discovers the already-running OpenCode service from its local registration and speaks API major version 2; it does not launch OpenCode. It accepts only a loopback service endpoint and authenticates with the registration password. Review delivery can select an existing session or create one when there is no target session. Session activity shown alongside terminals is associated by terminal title and foreground process, not a durable shell session identifier.

Sources: [Sidebar](../src/components/Sidebar.vue), [session presentation](../src/presentation/agent-sessions.ts), [review composer](../src/components/ReviewComposer.vue), [OpenCode discovery/version check](../src-tauri/src/services/opencode.rs), [agent bridge](../src-tauri/src/services/agent.rs).

### Integrated terminals

Open multiple shell terminals in a checkout and interact with them in the workspace. Terminal settings include font size, ligatures, blinking and cursor shape, scrollbar visibility, shell integration, and worktree-following behavior. Following can be disabled or triggered by shell navigation, OpenCode activity, or either.

Terminals are runtime processes: Muster does not restore them as running sessions after an application restart.

Sources: [TerminalSession](../src/components/TerminalSession.vue), [terminal setup](../src/lib/muster-terminal.ts), [terminal service](../src-tauri/src/services/terminal.rs), [persistence startup behavior](../src-tauri/src/persistence/mod.rs).

### Layout and preferences

The navigation, document, and inspector areas can be resized, and the workspace offers split and focused layout modes. Muster remembers workspace and checkout UI state. Preferences are stored separately in ~/.muster/config.yml.

| Area      | Default                                                                                                        | Available settings                                                                                                                                             |
| --------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Interface | 14 px, 100% scale, system theme, #16181c content background, sticky folders on                                 | Font size 11–20 px; scale 80–150% in 10 percentage-point steps; system/light/dark theme; content background; sticky folder rows                                |
| Terminal  | 16 px; ligatures and blinking on; block cursor; hidden scrollbar; shell integration on; follows agent activity | Font size 9–32 px; ligatures; cursor blink and block/bar/underline shape; hidden/automatic/always scrollbar; shell integration; follow off, cd, agent, or both |
| Editor    | 13 px; ligatures and blinking on; spaces, 2 columns                                                            | Font size 9–32 px; ligatures; cursor blink; tabs or spaces; indentation 1–8 columns                                                                            |

Sources: [App shell](../src/App.vue), [UI state](../src/domain/ui-state.ts), [layout persistence](../src/presentation/layout-persistence.ts), [settings model and defaults](../src/domain/settings.ts), [zoom options](../src/domain/zoom.ts).

## Granular behavior audit

The summaries above are backed by the following narrower behaviors. These distinctions are intentional: backend support, stored data shapes, and design documents do not establish that a control is present in the current UI.

### Repository and file navigation

- Open directory uses a single-folder picker and registers a Git repository/worktree or a plain folder. The titlebar workdir menu lists checkouts already in this window, recent paths not already open, and Open directory; opening a recent path registers it in the workspace. Repository groups can be collapsed. Their action menu creates worktrees, restores archived worktrees, or removes the repository from the panel. Removing a primary checkout or plain folder deletes its workspace registration and cascaded Muster state for those checkouts, including review notes/rounds and saved UI state, but leaves repository files untouched; the Home checkout cannot be removed. This is not an archive or an undo, so export needed notes or back up the database first. Opening it again creates a fresh registration; previously deleted attached notes and rounds do not return.
- Git repositories group their primary checkout and worktrees; plain folders have no Git worktree actions. Creation accepts a task name and new branch, fixes the directory to `repo/.worktrees`, and always branches from local main or origin/main; it is not an arbitrary location/base picker. Creation requires main to exist and refuses a branch already checked out. The .worktrees directory is added to Git's exclude file.
- A worktree can be archived to hide it from the sidebar while retaining its files, branch, commits, registration, and attached Muster review/UI state for a later restore. One repository action restores all its archived worktrees. Archive is unavailable while shell or agent sessions are active. Delete removes the worktree directory (or prunes metadata for a missing worktree) and its attached Muster state, including review notes/rounds: dirty files require explicit confirmation, active shell sessions are listed and their exact set confirmed before they are stopped, and active OpenCode sessions must be stopped first. The dialog reports commits not merged into the repository default branch; branch deletion is opt-in and can discard those commits. The primary checkout cannot be deleted.
- Missing checkouts are represented as missing and cannot be selected. A user can close a missing row to clear its registration and cascaded Muster state, including attached review notes/rounds; repository/worktree files are not deleted, and Muster does not automatically discover or reattach moved folders. The backend locate command is not exposed as a UI action.
- The Files tree loads directories as they are expanded, retains expanded paths and scroll position per checkout, windows large row lists, and can pin ancestor folders while scrolling. Dotfiles and Git-ignored entries are visually subdued. Empty, loading, missing, permission and truncated-list states are presented in the tree. The tree exposes expand/select rather than in-tree search or filesystem context-menu actions; the document toolbar has a copy-path action.
- The Changes list groups paths by parent directory, preserves rename source paths, displays Git status and available added/deleted counts, and includes untracked files. Selecting a file opens its diff; All changes opens the stacked change set. Git status events refresh the tree and change list.

Sources: [Sidebar](../src/components/Sidebar.vue), [worktree dialog](../src/components/WorktreeDialog.vue), [workspace and worktree services](../src-tauri/src/services/workspace.rs), [worktree service](../src-tauri/src/services/worktree.rs), [persistence cascades](../src-tauri/src/persistence/mod.rs), [workspace data and close behavior](README.md), [InspectorPane](../src/components/InspectorPane.vue), [file service](../src-tauri/src/services/files.rs).

Tests: [App.test.ts](../src/App.test.ts) covers recent-project menu behavior and confirmed archive restoration; [Sidebar.test.ts](../src/components/Sidebar.test.ts) checks repo grouping, checkout selection, terminal rows, and worktree actions; [WorktreeDialog.test.ts](../src/components/WorktreeDialog.test.ts) covers main-based creation, fixed location, archive/delete choice, and removal confirmations; inline tests in [worktree.rs](../src-tauri/src/services/worktree.rs) cover fixed main/location, dirty confirmation, unmerged commits, branch deletion, primary protection, and terminal confirmation; [workspace.rs](../src-tauri/src/services/workspace.rs) covers archive/restore and close registration; [InspectorPane.test.ts](../src/components/InspectorPane.test.ts) covers tree states, hidden/ignored styling, virtualization, sticky ancestors, change grouping/counts, and refresh; [workspace.test.ts](../src/domain/workspace.test.ts) covers checkout identity and terminal-to-worktree resolution; [ui-state.test.ts](../src/domain/ui-state.test.ts) covers saved paths and layout normalization.

### Documents, editing and previews

- A selected file opens in the central document. Markdown defaults to View and can switch between View and Code; other source documents open as code. CodeMirror supplies editable text, line numbers, undo history, syntax grammars, configured indentation, changed-line markers, a fold gutter/keymap, bracket matching and closing, rectangular selection and multiple cursors. The searchable language picker can override auto-detection for the Code view.
- Prettier is offered only for languages with an available parser, using the checkout's Prettier configuration. Formatting creates an unsaved draft; Save writes it and Cancel discards it. The save path protects against writing over file contents that changed since they were read. Document identity keeps checkout files and exported documents with the same name separate.
- Markdown renders GFM tables, task lists, headings, fenced code and top-of-file YAML front matter. Front matter is shown as an expandable YAML tree. Relative document links are resolved within the active checkout; web links are handed to the system browser. Local preview images must resolve inside the checkout and fit preview limits. Remote images are not fetched. Rendered output is sanitized and raw HTML is disabled.
- Dedicated media previews cover PNG, JPG/JPEG, GIF, WebP, AVIF, ICO, BMP, MP4, WebM, MOV, OGV, and SVG. SVG is parsed and displayed as an image, including an unsaved XML draft; it is not inserted as active page markup. PDF and audio preview are not supported.
- The document pane has no separate breadcrumb or general find/replace, lint, or completion panel. The titlebar still provides path crumbs; document controls are the language picker, Markdown View/Code, formatting, Save and Cancel.

Sources: [DocumentPane](../src/components/DocumentPane.vue), [CodeMirror setup](../src/lib/code-editor.ts), [language mappings](../src/lib/source-languages.ts), [Prettier integration](../src/lib/prettier-format.ts), [media classification](../src/domain/media.ts), [Markdown renderer](../src/lib/markdown-preview.ts), [Markdown presentation](../src/presentation/markdown-preview.ts), [front matter](../src/lib/front-matter.ts), [file commands](../src-tauri/src/commands/files.rs).

Tests: [DocumentPane.test.ts](../src/components/DocumentPane.test.ts) covers mode selection, media refresh/disposal, safe links, path copying, drafts, formatting and SVG; [code-editor.test.ts](../src/lib/code-editor.test.ts) covers gutter/undo, indentation, changed-line markers and multi-cursor state; [source-languages.test.ts](../src/lib/source-languages.test.ts) covers language detection; [markdown-preview.test.ts](../src/lib/markdown-preview.test.ts) covers rendering, image containment and sanitization; [front-matter.test.ts](../src/lib/front-matter.test.ts) covers metadata fences; [changed-lines.test.ts](../src/lib/changed-lines.test.ts) covers mapping changed lines.

### Git status and diff presentation

- Every Git checkout uses the same repository default base: the persisted default branch, otherwise origin/HEAD, otherwise the primary checkout's current branch. When no default branch is saved for the repository, Muster prompts for one; the selected branch must exist locally or as an origin ref. For each checkout, tracked committed differences are compared from merge-base(default ref, HEAD) to HEAD, alongside staged/unstaged status and untracked files; line counts use the same base. Thus a primary checkout on its default branch usually represents ordinary git status, while a primary on another branch or a worktree can include committed branch changes since the common ancestor. These counts are not limited to uncommitted changes or to the worktree's creation point. Binary files have no fabricated zero counts; rename rows retain the old path. Diff rendering is unified, with foldable hunks and no side-by-side mode control. All changes is a stack of per-file diffs, with each file read when expanded.
- Large diffs are virtualized and fetched in bounded pages around the viewport. Pages and in-flight requests are capped; stale responses for a different checkout, path or revision are discarded. Binary, symlink and over-limit changes receive explicit non-text states rather than being treated as empty text. Safe preview ceilings include 100,000 lines, 10,000 hunks, 32 MiB, 4 KiB hunk headers and 64 KiB per line.
- The diff uses the editor font setting. Note range selection is available on unified lines; a range is refused if required code was not loaded, rather than exporting a guessed snippet. Git watcher revisions prevent unchanged updates from needlessly rebuilding the open diff.

Sources: [InspectorPane](../src/components/InspectorPane.vue), [FileDiff](../src/components/FileDiff.vue), [large-diff paging](../src/components/use-large-diff.ts), [diff loader](../src/presentation/diff-loader.ts), [default-branch prompt](../src/presentation/workspace.ts), [workspace service](../src-tauri/src/services/workspace.rs), [Git service and limits](../src-tauri/src/services/git.rs).

Tests: [InspectorPane.test.ts](../src/components/InspectorPane.test.ts) covers change rows and counts; [FileDiff.test.ts](../src/components/FileDiff.test.ts) covers hunk folding, unified rendering, refresh identity and review ranges; [use-large-diff.test.ts](../src/components/use-large-diff.test.ts) covers viewport paging/windowing; [diff-loader.test.ts](../src/presentation/diff-loader.test.ts) covers diff refresh and stale reads; inline Git service tests cover merge-base files/counts and default-ref resolution.

### Review-note states and delivery

- A note records checkout, path, old/new side, a line or range, text, and the code/hash captured when it was created. Its status is draft, sent or resolved; outdated is a separate sticky flag. Drift is judged only where loaded diff lines can answer it. Unknown is not treated as unchanged. A user can clear the outdated flag; outdated notes are withheld from delivery unless the inclusion control is selected.
- Notes can be edited or deleted. Resolution is offered only for an already-sent, non-outdated note whose current anchor the diff can verify. Markdown export uses the captured note data and can include ranges without requiring an OpenCode target.
- OpenCode delivery groups selected notes into a persisted round for one chosen session. If that agent is busy, the user can queue instead of sending immediately. Rounds are recorded with their exact prompt and marker before dispatch; notes are linked and marked sent at that point. Reconciliation checks the marker in the session transcript: a landed send is confirmed, an absent send can be queued for retry, and an uncertain send is not blindly duplicated. Queued rounds flush when the agent becomes available. Acknowledging a finished agent turn closes the round; it does not resolve its notes.

Sources: [review domain](../src/domain/review.ts), [FileDiff](../src/components/FileDiff.vue), [note cards](../src/components/ReviewNoteList.vue), [review composer](../src/components/ReviewComposer.vue), [review-note store](../src/presentation/review-notes.ts), [review-round service](../src-tauri/src/services/review_round.rs), [review persistence](../src-tauri/src/persistence/mod.rs).

Tests: [review.test.ts](../src/domain/review.test.ts) covers Markdown export, note selection and anchor outcomes; [review-notes.test.ts](../src/presentation/review-notes.test.ts) covers persistence, edits, drift, queue/flush/reconciliation and acknowledgement; [FileDiff.test.ts](../src/components/FileDiff.test.ts) covers note creation, sending, outdated-note gating and resolution; inline tests in [review_round.rs](../src-tauri/src/services/review_round.rs) cover record-before-send, markers, checkout ownership, retry and prompt bounds.

### OpenCode service boundary

The OpenCode bridge calls an already-running local API v2 service; Muster does not spawn the OpenCode executable or service. The sidebar can create/select sessions and reflect their activity. Terminal-to-session association is inferred from the OpenCode terminal title and current foreground process, not a durable shell session identifier; ambiguous or missing matches are not presented as confirmed associations.

Sources: [agent-session presentation](../src/presentation/agent-sessions.ts), [OpenCode API client](../src-tauri/src/services/opencode.rs), [agent bridge](../src-tauri/src/services/agent.rs), [Sidebar](../src/components/Sidebar.vue).

Tests: [Sidebar.test.ts](../src/components/Sidebar.test.ts) covers terminal-title matching, truncated/duplicate names and refusal to borrow another terminal's match; [agent-sessions.test.ts](../src/presentation/agent-sessions.test.ts) covers session list behavior.

### Terminal sessions: what is and is not in the UI

- Multiple shell processes may be created for a checkout. They are listed as selectable terminal rows in the sidebar; the main pane mounts the selected terminal. Rows support user renaming and closing. A terminal can be dragged/reordered or moved to another registered worktree in the same repository without restarting its PTY. A manual move sends cd only when the shell is idle; a running foreground process is left alone. Automatic follow can respond to shell cd, OpenCode activity, both, or neither; an automatically moved terminal does not steal the current view unless it was already selected.
- Closing a terminal asks before stopping a foreground process and closes an idle shell without prompting. App close also asks before terminating active foreground processes. Terminal output uses a bounded, ordered channel; input, resize, and process status are scoped to the owning session. Supported shell integration reports command exit status for zsh and bash while preserving normal startup behavior; it applies to new terminals.
- Terminal tabs and split-terminal panes are not current UI features. Although a terminal-layout data type and backend load/save commands exist, the pane normalizes a saved layout to one live session and does not restore historical PTYs. The workspace itself does have split/focused panel layouts; that is distinct from splitting terminals.
- Terminal output can open a checkout file after a validated path probe on the platform modifier-click, or open a web URL in the system browser. Its custom scrollbar supports hidden, while-scrolling and always modes; a WebGL renderer failure falls back to the normal renderer.

Sources: [SessionPane](../src/components/SessionPane.vue), [TerminalSession](../src/components/TerminalSession.vue), [terminal service](../src-tauri/src/services/terminal.rs), [terminal layout normalization](../src/domain/terminal-layout.ts), [shell integration parser](../src/lib/terminal-shell-integration.ts), [terminal file links](../src/lib/terminal-file-links.ts), [terminal path parsing](../src/lib/terminal-paths.ts), [terminal keys](../src/lib/terminal-keys.ts).

Tests: [Sidebar.test.ts](../src/components/Sidebar.test.ts) covers selectable, renameable, closable terminal rows and exit status; [SessionPane.test.ts](../src/components/SessionPane.test.ts) covers creating multiple sessions, moving/reordering and collapsing legacy split layouts; [TerminalSession.test.ts](../src/components/TerminalSession.test.ts) covers PTY I/O, close confirmation, resize/polling, output flow and renderer recovery; [terminal-layout.test.ts](../src/domain/terminal-layout.test.ts) covers persisted layout normalization; [terminal-file-links.test.ts](../src/lib/terminal-file-links.test.ts), [terminal-paths.test.ts](../src/lib/terminal-paths.test.ts) and [terminal-keys.test.ts](../src/lib/terminal-keys.test.ts) cover path, URL and key protocol handling; [terminal-shell-integration.test.ts](../src/lib/terminal-shell-integration.test.ts) covers OSC parsing; inline tests in [terminal.rs](../src-tauri/src/services/terminal.rs) exercise zsh/bash hooks and startup preservation.

### App shell, layout, settings and accessibility

- The titlebar has searchable crumb menus for checkout/session navigation and a separate Settings action. Its Files/Changes inspector, central document/diff, and terminal areas resize and support workspace split/focused layouts. The titlebar menu search filters labels and hints while keeping pinned actions available.
- Window layout and per-checkout document, inspector tab, expansion and scroll state are persisted locally. Settings are a separate hand-editable YAML file under the user's home. Settings edits are drafts until Apply; reset can restore one field or all defaults. The schema normalizes malformed ranges and enum values. System/light/dark theme, interface scale, terminal and editor preferences are exposed in Settings.
- Window shortcuts use Command on macOS and Ctrl elsewhere: modifier+/ toggles side panels, modifier+N requests a terminal for the active checkout, modifier+plus/minus changes scale and modifier+0 resets it. Settings lists these chords; contextual actions include Escape, Enter, Home/End and modifier-click behavior. The window close path flushes queued UI/settings writes and asks before terminating running commands.
- Modal dialogs name their purpose, move focus inside, trap Tab, support Escape where appropriate and restore focus to their trigger. The Tauri menu provides About and Edit actions; the generic WebView right-click menu is suppressed to prevent accidental reloads that would discard drafts. Frontend diagnostics are sent under fixed categories; app logs are written to rotating log targets.
- The WebView CSP disallows object embedding and remote scripts; file services validate checkout-relative paths, and Markdown HTML is sanitized. These are implementation boundaries, not a claim that arbitrary file content is trusted.

Sources: [App shell](../src/App.vue), [settings dialog](../src/components/SettingsDialog.vue), [settings schema](../src/domain/settings.ts), [shortcut model](../src/domain/shortcuts.ts), [layout state](../src/domain/ui-state.ts), [layout persistence](../src/presentation/layout-persistence.ts), [dialog focus helpers](../src/lib/dialog-focus.ts), [context-menu suppression](../src/lib/context-menu.ts), [native menu](../src-tauri/src/menus.rs), [app setup/logging](../src-tauri/src/main.rs), [WebView CSP](../src-tauri/tauri.conf.json), [file path validation](../src-tauri/src/services/files.rs).

Tests: [App.test.ts](../src/App.test.ts) covers close handling, layout, zoom, shortcuts, settings save/failure, theme and focus; [settings.test.ts](../src/domain/settings.test.ts) covers defaults and normalization; [shortcuts.test.ts](../src/domain/shortcuts.test.ts) and [zoom.test.ts](../src/domain/zoom.test.ts) cover platform chords and scale steps; [titlebar-menu.test.ts](../src/domain/titlebar-menu.test.ts) covers query filtering; [dialog-focus.test.ts](../src/lib/dialog-focus.test.ts) covers Tab trapping; [context-menu.test.ts](../src/lib/context-menu.test.ts) covers right-click suppression; [ui-state.test.ts](../src/domain/ui-state.test.ts) covers persisted layout and path validation.

### Audit scope

The catalog was checked against user-facing UI and presentation flows, relevant Rust IPC/services and persistence, tests, documentation, configuration, and release-target files. Tests are linked as evidence; they were not executed for this documentation audit. This is a feature inventory, not a test-pass or coverage report.

## Local data and startup

Workspace, checkout UI, and review data are stored locally; settings live in the local configuration file. Muster restores the workspace but does not restart terminal processes. Explicitly closing/removing a registration or deleting a worktree cascades away its associated persisted Muster data, including notes and review rounds, while leaving repository files untouched unless the worktree Delete action is chosen. Archive preserves that local state. The SQLite schema uses a strict version check and has no migration ladder: a database stamped with an unrecognized schema version is refused rather than upgraded.

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

## Development and release tooling (not app features)

- package.json defines the Vite/Tauri development and build commands, formatting, lint/type-check, frontend/Rust/release/artifact tests, security audits, and the local release validation gate. The aggregate pnpm test command runs the frontend tests, release-script tests, audit tests, Rust tests and artifact tests; this catalog task did not run them.
- scripts/release.mjs checks version agreement across the four version records and can bump, dry-run, or restore a lower v0 semantic version. It only writes version files; it does not commit or create tags.
- scripts/release.sh requires a clean main checkout, prepares the version, runs release validation, commits only the version files, pushes the validated commit, and hands builds/publication to GitHub Actions. Its dry-run requests validation-only builds and does not create a tag/release, although the prepared commit may be pushed.
- CI runs checks for pull requests and main pushes on macOS plus a Linux packaging job. The release workflow builds/verifies configured artifacts. These are development and distribution mechanisms, not functionality exposed in the desktop UI.

Sources: [package scripts](../package.json), [version tool](../scripts/release.mjs), [release driver](../scripts/release.sh), [CI workflow](../.github/workflows/ci.yml), [release workflow](../.github/workflows/release.yml), [Tauri package targets](../src-tauri/tauri.conf.json).

## Specified, not implemented

The following review destinations are planned in design documents, but are not available in the shipped UI:

- [Codex](todo/phase-2-codex.md)
- [Claude Code](todo/phase-3-claude-code.md)
- [pi](todo/phase-4-pi.md)

## Catalog files

[features.json](features.json) is the machine-readable counterpart to this document. It contains no screenshot selectors, fixture assumptions, or capture storyboard. Source tests were inspected during the audit but are not claimed as executed.
