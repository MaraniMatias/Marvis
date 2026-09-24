# Marvis — Product & Technical Requirements

> Local developer workspace for terminals, Git, files, reviews and coding agents.

**Status:** Draft v0.2
**Application:** Marvis
**Initial platform:** macOS
**Frontend:** Vue 3 + TypeScript + Vite + reka-ui (ver si tiene sentido @git-diff-view/vue)
**Desktop runtime:** Tauri 2
**Native core:** Rust
**Primary agent:** OpenCode

---

# 1. Vision

Marvis is a local desktop application for supervising development work organized around real filesystem directories.

The central unit in Marvis is the **directory**.

Everything else belongs to that directory:

```text
Directory
├── Sessions
│   ├── Shell
│   ├── OpenCode
│   ├── Neovim
│   └── Development servers
│
├── Files
├── Git Context
├── Changes
├── Review
├── Preview
└── Activity
```

A Git repository is a directory with Git capabilities.

A Git worktree is a directory with Git capabilities associated with another repository.

A normal folder is still a valid Marvis directory, but without Git capabilities.

Marvis should make it possible to answer four questions immediately:

```text
Sidebar     → Where am I working?
Center      → What is running there?
Inspector   → What exists / changed there?
Status Bar  → What is happening now?
```

---

# 2. Product Goal

Current development workflows are fragmented across:

- Terminal windows
- Terminal tabs
- Git diff tools
- Editors
- Markdown viewers
- Image viewers
- AI coding agents
- Git worktrees
- Development servers

This becomes particularly difficult when several AI agents are working simultaneously in different worktrees.

Marvis provides one local control plane for that workflow.

The core loop is:

```text
Directory
   ↓
Start Agent
   ↓
Agent modifies files
   ↓
Git detects changes
   ↓
Developer reviews diff
   ↓
Developer writes review comments
   ↓
Send review to agent
   ↓
Agent continues
   ↓
Review again
```

---

# 3. Product Principles

## 3.1 Directory is the source of context

The active directory determines:

- Sessions
- Files
- Git repository
- Branch
- Worktree
- Changes
- Review
- Preview
- Agent state

The inspector MUST NOT have an independent working directory.

---

## 3.2 Sessions belong to directories

A terminal, OpenCode agent or Neovim instance is not a global tab.

It belongs to a directory.

```text
ledger-api
├── shell
│
├── billing
│   ├── OpenCode
│   ├── shell
│   └── nvim
│
└── migrate
    ├── OpenCode
    └── shell
```

---

## 3.3 Capabilities are detected

The user should not manually configure whether a directory is:

- Plain folder
- Git repository
- Git worktree

Marvis detects its capabilities.

---

## 3.4 Agent supervision is first-class

OpenCode is not a chat sidebar.

Agent state is part of the directory state.

Marvis must make it immediately visible when an agent is:

```text
working
waiting for input
waiting for permission
waiting for review
completed
failed
```

---

## 3.5 Marvis does not replace the editor

Marvis is responsible for:

- Navigation
- Terminal
- Git
- Diff
- Review
- Preview
- Agent supervision

Editing source code remains the responsibility of tools such as:

- Zed
- Neovim

---

## 3.6 Local-first

Core functionality MUST work locally.

No Marvis cloud service is required.

---

## 3.7 Performance is a feature

Marvis should feel closer to a terminal than to a browser-based IDE.

Priorities:

```text
low input latency
fast startup
low idle CPU
bounded memory usage
smooth resizing
fast workspace switching
no UI blocking during Git/filesystem operations
```

---

# 4. Non-Goals for v1

Marvis v1 will NOT include:

- Built-in source code editor
- GitHub/GitLab pull requests
- Issues
- Cloud synchronization
- Collaboration
- Plugin marketplace
- Remote development
- SSH workspaces
- Agents other than OpenCode
- Full IDE functionality

---

# 5. Technology Stack

```text
Desktop
└── Tauri 2

Frontend
├── Vue 3
├── TypeScript
└── Vite

Native Core
└── Rust

Terminal PTY
└── portable-pty initially

Terminal Renderer
└── adapter-based
    ├── WebView renderer initially
    └── libghostty experiment later

Git
└── Git CLI

Persistence
└── SQLite

Agent
└── OpenCode

Editors
├── Zed
└── Neovim
```

---

# 6. Architectural Boundary

Marvis has two main layers.

```text
┌───────────────────────────────────────────────┐
│                 MARVIS UI                     │
│                                               │
│        Vue 3 + TypeScript + Vite              │
│                                               │
│ Sidebar                                       │
│ Session Layout                                │
│ Inspector                                     │
│ Diff                                          │
│ Preview                                       │
│ Agent UI                                      │
│ Command Palette                               │
└──────────────────────┬────────────────────────┘
                       │
                Tauri IPC
          Commands / Channels
                       │
┌──────────────────────▼────────────────────────┐
│                MARVIS CORE                    │
│                     Rust                      │
│                                               │
│ DirectoryService                              │
│ SessionService                                │
│ TerminalService                               │
│ GitService                                    │
│ FileService                                   │
│ AgentService                                  │
│ EditorService                                 │
│ ReviewService                                 │
│ PersistenceService                            │
│ ActivityService                               │
└──────┬──────┬──────┬──────┬──────┬──────────┘
       │      │      │      │      │
      PTY    Git   Files  OpenCode Editors
```

Vue MUST NOT directly execute:

```text
git
opencode
nvim
zed
shell commands
filesystem mutations
```

Those operations belong to Rust services.

---

# 7. IPC Strategy

Use two kinds of Tauri communication.

## Commands

Commands are for request/response operations.

Examples:

```text
directory.open
directory.close

git.status
git.diff

files.list

agent.create

editor.open
```

Conceptually:

```text
Vue
 ↓
invoke()
 ↓
Tauri command
 ↓
Rust service
 ↓
result
```

---

## Channels

Channels are for high-volume or continuous streams.

Examples:

```text
PTY output
Agent event streams
Long-running process output
Potential large incremental operations
```

Most importantly:

```text
PTY
 ↓
Rust
 ↓
Tauri Channel<ArrayBuffer>
 ↓
Terminal Renderer
```

PTY output MUST NOT be implemented as thousands of global UI events.

---

# 8. Core Domain Model

## Directory

```ts
interface Directory {
  id: string;

  path: string;
  canonicalPath: string;

  type: "plain" | "repository" | "worktree";

  git?: GitContext;

  sessions: Session[];

  createdAt: string;
  lastOpenedAt: string;
}
```

Identity is based on canonical filesystem path.

Opening the same canonical path twice MUST focus the existing directory.

---

# 9. Git Context

```ts
interface GitContext {
  repositoryRoot: string;

  branch?: string;

  head?: string;

  isWorktree: boolean;

  mainWorktree?: string;

  baseBranch?: string;

  changedFiles: number;
}
```

Marvis MUST distinguish:

```text
selected directory
repository root
worktree root
```

They are not necessarily identical.

Git itself should be the source of truth for repository/worktree detection.

---

# 10. Directory Capability Matrix

| Capability       | Plain | Repository | Worktree |
| ---------------- | ----: | ---------: | -------: |
| Terminal         |     ✓ |          ✓ |        ✓ |
| Files            |     ✓ |          ✓ |        ✓ |
| Preview          |     ✓ |          ✓ |        ✓ |
| Zed              |     ✓ |          ✓ |        ✓ |
| Neovim           |     ✓ |          ✓ |        ✓ |
| Git status       |       |          ✓ |        ✓ |
| Branch           |       |          ✓ |        ✓ |
| Diff             |       |          ✓ |        ✓ |
| Review           |       |          ✓ |        ✓ |
| Agent            |     ✓ |          ✓ |        ✓ |
| Create worktree  |       |          ✓ |          |
| Worktree nesting |       |     parent |    child |

---

# 11. Application Layout

Desktop layout:

```text
┌─────────────────────────────────────────────────────────────┐
│ ~/dev/ledger-api/.worktrees/billing             ⌘K        │
├──────────────┬───────────────────────────┬──────────────────┤
│              │                           │                  │
│  Sidebar     │       Sessions            │    Inspector     │
│              │                           │                  │
│ ledger-api   │   OpenCode / Terminal     │ Files            │
│ ├ billing    │                           │ Changes          │
│ ├ migrate    │                           │ Preview          │
│ └ docs-api   │                           │                  │
│              │                           │                  │
├──────────────┴───────────────────────────┴──────────────────┤
│ agent/billing │ 2 changes │ agent: running                 │
└─────────────────────────────────────────────────────────────┘
```

---

# 12. Sidebar

The sidebar represents **places**, not tabs.

Repository example:

```text
ledger-api                 main
│
├─ billing                 +42
│   ├─ opencode            ●
│   └─ zsh
│
├─ migrate                 +118
│   └─ opencode            ●
│
└─ docs-api                idle
```

Plain directory:

```text
notas
├─ zsh
└─ nvim
```

---

# 13. Attention States

Directory/session indicators:

```text
green   running / healthy
blue    agent actively working
amber   user attention required
red     error
none    idle
```

Amber includes:

- Agent finished and waiting for review
- Permission required
- Unsent review comments
- Agent waiting for user input

The sidebar should behave partly as an **attention inbox**.

---

# 14. Sessions

```ts
type SessionType = "shell" | "agent" | "nvim" | "server" | "custom";

interface Session {
  id: string;
  directoryId: string;
  type: SessionType;
  title: string;
  status: SessionStatus;
}
```

A directory may contain multiple sessions.

Changing session MUST NOT change directory context.

Therefore:

```text
activeDirectory
```

and:

```text
activeSession
```

are separate state variables.

---

# 15. Session Layout

Sessions support:

```text
tabs
horizontal splits
vertical splits
nested splits
```

Example:

```text
billing

┌────────────────────────┬───────────────────────┐
│                        │                       │
│ OpenCode               │ nvim                  │
│                        │                       │
├────────────────────────┴───────────────────────┤
│ dev server                                     │
└────────────────────────────────────────────────┘
```

Layout MUST persist.

---

# 16. Terminal Architecture

Terminal architecture:

```text
Vue Terminal Component
        │
        │ input
        ▼
Tauri Command
        │
        ▼
TerminalService
        │
        ▼
portable-pty
        │
        ▼
zsh / fish / bash / nvim / OpenCode
        │
        │ output bytes
        ▼
Tauri Channel
        │
        ▼
Terminal Renderer
```

`portable-pty` provides the initial native PTY abstraction.

Terminal rendering is independent from PTY management.

---

# 17. TerminalAdapter

Rust:

```rust
trait TerminalBackend {
    fn create(&self, options: TerminalOptions) -> Result<TerminalId>;
    fn write(&self, id: TerminalId, data: &[u8]) -> Result<()>;
    fn resize(&self, id: TerminalId, cols: u16, rows: u16) -> Result<()>;
    fn kill(&self, id: TerminalId) -> Result<()>;
}
```

This boundary allows implementation changes without modifying product logic.

---

# 18. Terminal Requirements

Terminal MUST support:

```text
real PTY
ANSI escape sequences
Unicode
true color
resize
SIGWINCH-equivalent resize behavior
scrollback
mouse
clipboard
alternate screen
bracketed paste
OSC sequences where appropriate
shell applications
```

Must work correctly with:

```text
zsh
fish
bash
nvim
vim
htop
git
OpenCode
interactive CLIs
```

---

# 19. Terminal Renderer

The renderer MUST be treated separately from the PTY.

Initial architecture:

```text
TerminalService
       │
       ▼
TerminalRenderer
```

Possible implementations:

```text
XtermRenderer
GhosttyRenderer
```

v1 SHOULD begin with the lowest-risk renderer that meets performance requirements.

Do not couple the rest of Marvis to xterm or Ghostty.

---

# 20. libghostty

libghostty is an optimization/UX investigation, not a v1 architectural requirement.

A technical spike SHOULD determine whether it can provide enough benefit in:

```text
rendering performance
font rendering
ligatures
terminal compatibility
latency
GPU rendering
Ghostty-like experience
```

without forcing Marvis into excessive native UI complexity.

Marvis MUST remain functional without libghostty.

---

# 21. Git Architecture

Use the installed Git executable as the primary Git engine.

```text
Vue
 ↓
GitService
 ↓
git CLI
```

Do NOT introduce libgit2 initially.

Advantages:

```text
same Git behavior developer expects
worktree support
Git configuration compatibility
credential/helper compatibility
lower implementation complexity
```

---

# 22. Git Detection

Use Git commands as source of truth.

Required information:

```text
repository root
worktree root
branch
HEAD
worktree relationships
dirty state
```

Useful commands include:

```bash
git rev-parse
git status --porcelain=v2
git worktree list --porcelain
```

Do not rely solely on manually parsing `.git`.

---

# 23. Git Status

Git status MUST update after:

```text
agent modifies files
editor modifies files
terminal modifies files
checkout
commit
merge
rebase
worktree changes
```

Architecture:

```text
Filesystem Watcher
       │
       ▼
debounce
       │
       ▼
GitService
       │
       ▼
git status --porcelain=v2
       │
       ▼
GitState
```

---

# 24. Git Diff

Inspector → Changes:

```text
Changes 2

src/
├─ billing.ts      M
└─ billing.test.ts M
```

Selecting a file loads only that file's diff.

Do NOT load/render an entire enormous repository diff unnecessarily.

---

# 25. Diff Requirements

Support:

```text
unified diff
file-by-file loading
collapsible hunks
syntax highlighting
line numbers
added/deleted lines
viewed state
comments
```

Later:

```text
side-by-side diff
stage hunk
unstage hunk
```

Large diffs require:

```text
lazy loading
virtualization
file-level limits
hunk-level rendering
binary detection
```

A 4,000+ line diff MUST NOT freeze the application.

---

# 26. Inspector

The right inspector is always a projection of:

```text
activeDirectory
```

Tabs:

```text
Files
Changes
Preview
```

Capabilities determine available tabs.

Plain folder:

```text
Files
Preview
```

Repository/worktree:

```text
Files
Changes
Preview
```

Unavailable tabs SHOULD NOT be rendered.

---

# 27. Files

Files view MUST support:

```text
directory tree
Git decorations
.gitignore-aware behavior
modified indicators
file selection
fuzzy file search
open in editor
preview
```

Large trees MUST use virtualization.

Do not render tens of thousands of DOM nodes.

---

# 28. Preview

Supported v1:

```text
Markdown
Images
Text
Source code
JSON
YAML
```

Preview is for understanding files.

It is NOT an editor.

---

# 29. Markdown Preview

Required:

```text
CommonMark/GFM-style rendering
tables
task lists
code blocks
syntax highlighting
relative images
safe HTML handling
```

Markdown content MUST be sanitized before injecting HTML into the WebView.

---

# 30. Image Preview

Support:

```text
PNG
JPEG
WebP
GIF
SVG when safely handled
```

Features:

```text
fit
100%
zoom
dimensions
open externally
```

Large images SHOULD NOT block the UI thread.

---

# 31. Review System

Review is local Marvis state.

It is not GitHub review.

Flow:

```text
Agent finishes
      ↓
Changes
      ↓
Developer reviews
      ↓
Draft comments
      ↓
Send review
      ↓
Agent receives one review message
      ↓
Agent continues
```

---

# 32. Review Comments

```ts
interface ReviewComment {
  id: string;

  directoryId: string;

  filePath: string;

  side: "old" | "new";

  startLine: number;
  endLine?: number;

  contentHash?: string;

  body: string;

  status: "draft" | "sent" | "resolved" | "outdated";

  createdAt: string;
}
```

---

# 33. Comment Reanchoring

Comments MUST NOT rely exclusively on line number.

Persist:

```text
file
side
line/range
nearby content
content hash
```

If a subsequent agent modification moves the code:

```text
attempt reanchor
```

If confidence is insufficient:

```text
mark comment as outdated/unanchored
```

Never silently attach a comment to unrelated code.

---

# 34. Batch Review

Draft comments are sent together.

Example:

```text
Review feedback:

src/billing.ts:43
Backoff debería incluir jitter.

src/billing.ts:71
Este error debería conservar el cause original.

tests/billing.test.ts:118
Agregá el caso de timeout.
```

One action:

```text
Send review
```

Agent interruptions should be deliberate rather than sending a message for every comment.

---

# 35. OpenCode Architecture

OpenCode integration lives behind:

```rust
trait AgentBridge {
    fn create_session(...);
    fn send_message(...);
    fn cancel(...);
    fn respond_permission(...);
    fn subscribe(...);
}
```

Implementation:

```text
OpenCodeAgentBridge
```

Marvis domain code MUST NOT be named around OpenCode.

Use:

```text
Agent
AgentSession
AgentStatus
AgentEvent
AgentBridge
```

OpenCode is simply the first implementation.

---

# 36. Agent State

Internal states:

```text
starting
thinking
working
running_command
waiting_permission
waiting_input
waiting_review
completed
failed
```

UI MAY collapse these into:

```text
working
needs you
done
error
```

---

# 37. Agent Events

Normalize external events into Marvis events:

```ts
type AgentEvent =
  | { type: "status.changed"; status: AgentStatus }
  | { type: "file.active"; path: string }
  | { type: "file.changed"; path: string }
  | { type: "command.started"; command: string }
  | { type: "command.finished"; command: string; exitCode: number }
  | { type: "permission.requested"; permission: AgentPermission }
  | { type: "message"; content: string }
  | { type: "completed" }
  | { type: "failed"; error: string };
```

---

# 38. Follow Agent

Inspector footer:

```text
Follow Agent  [on/off]
```

When enabled:

```text
agent edits file
      ↓
AgentEvent::file.active
      ↓
Inspector selects file
      ↓
Preview / Changes follows it
```

Manual file selection pauses Follow Agent.

UI displays:

```text
Follow paused
Resume
```

When review begins, Follow Agent defaults to OFF.

---

# 39. Multiple Agents

Parallel worktrees are a core Marvis scenario.

Example:

```text
ledger-api

billing
└─ OpenCode      REVIEW

migrate
└─ OpenCode      WORKING

docs-api
└─ OpenCode      IDLE
```

Global status:

```text
2 agents active
billing: 2 unsent drafts
```

A background directory requiring attention MUST remain visible.

---

# 40. Editor Integration

Define:

```rust
trait EditorAdapter {
    fn available(&self) -> bool;

    fn open_workspace(&self, path: &Path) -> Result<()>;

    fn open_file(
        &self,
        path: &Path,
        line: Option<u32>,
        column: Option<u32>
    ) -> Result<()>;
}
```

Implement:

```text
ZedAdapter
NeovimAdapter
```

---

# 41. Zed

Required:

```text
open directory
open file
open file:line
open file:line:column
```

Marvis SHOULD detect whether the Zed CLI is available.

---

# 42. Neovim

Basic:

```text
nvim +<line> <file>
```

Advanced:

```text
Marvis
 ↓
Neovim RPC/socket
 ↓
existing Neovim instance
```

Future bidirectional integration MAY expose:

```text
current buffer
cursor
selection
current file
```

A future `marvis.nvim` plugin MAY provide richer integration.

---

# 43. File Watching

Rust owns filesystem watching.

```text
FileService
    │
    ├─ filesystem changes
    ├─ directory invalidation
    └─ Git invalidation
```

Events MUST be:

```text
debounced
coalesced
scoped by directory
```

File watcher storms MUST NOT produce equivalent UI event storms.

---

# 44. Persistence

Use SQLite for application state.

Persist:

```text
directories
directory ordering
sessions metadata
layout
splits
review comments
review rounds
preferences
recent paths
inspector state
window geometry
activity metadata
```

Do not persist terminal output indefinitely by default.

Do not duplicate OpenCode's entire conversation database.

---

# 45. State Ownership

Important rule:

```text
Rust owns system truth.
Vue owns presentation state.
SQLite owns persistent Marvis state.
Git owns Git truth.
OpenCode owns agent/session truth.
```

Examples:

Rust:

```text
PTY process
Git state
filesystem state
agent connection
process lifecycle
```

Vue:

```text
selected pane
hover state
temporary dialogs
animation
current command palette query
```

SQLite:

```text
directories
layout
draft reviews
preferences
```

---

# 46. Rust Core Structure

Recommended:

```text
src-tauri/src/

├── lib.rs
├── error.rs
├── state.rs
│
├── commands/
│   ├── directory.rs
│   ├── terminal.rs
│   ├── git.rs
│   ├── files.rs
│   ├── agent.rs
│   ├── editor.rs
│   └── review.rs
│
├── domain/
│   ├── directory.rs
│   ├── session.rs
│   ├── git.rs
│   ├── agent.rs
│   └── review.rs
│
├── services/
│   ├── directory_service.rs
│   ├── terminal_service.rs
│   ├── git_service.rs
│   ├── file_service.rs
│   ├── agent_service.rs
│   ├── editor_service.rs
│   ├── review_service.rs
│   └── persistence_service.rs
│
├── terminal/
│   ├── backend.rs
│   └── portable_pty.rs
│
├── agent/
│   ├── bridge.rs
│   └── opencode.rs
│
├── editor/
│   ├── adapter.rs
│   ├── zed.rs
│   └── neovim.rs
│
├── git/
│   ├── commands.rs
│   └── parser.rs
│
└── persistence/
    ├── database.rs
    └── migrations/
```

---

# 47. Vue Structure

```text
src/

├── App.vue
│
├── components/
│   ├── sidebar/
│   ├── sessions/
│   ├── terminal/
│   ├── inspector/
│   ├── files/
│   ├── diff/
│   ├── preview/
│   ├── agent/
│   ├── review/
│   └── common/
│
├── stores/
│   ├── directories.ts
│   ├── sessions.ts
│   ├── git.ts
│   ├── agents.ts
│   ├── review.ts
│   └── ui.ts
│
├── composables/
│   ├── useDirectory.ts
│   ├── useTerminal.ts
│   ├── useAgent.ts
│   └── useCommands.ts
│
├── tauri/
│   ├── commands.ts
│   ├── terminal.ts
│   └── channels.ts
│
├── types/
│
└── styles/
```

---

# 48. Frontend Rule

Never:

```ts
Command.create("git", ...)
```

Never expose:

```text
shell.exec(arbitraryString)
```

as the normal frontend architecture.

Instead:

```ts
await marvis.git.status(directoryId);

await marvis.editor.open({
  file,
  line,
});

await marvis.agent.sendReview({
  sessionId,
  comments,
});
```

The frontend speaks Marvis domain language.

---

# 49. Command Palette

Shortcut:

```text
⌘K
```

Search across:

```text
directories
recent paths
sessions
files
commands
actions
```

Examples:

```text
Open Directory
New Terminal
New OpenCode Session
New Worktree for Agent
Open File
Open Changes
Open Preview
Open in Zed
Open in Neovim
Send Review
Toggle Follow Agent
```

---

# 50. Opening a Directory

Flow:

```text
⌘K
 ↓
fuzzy path picker
 ↓
select directory
 ↓
canonicalize path
 ↓
already open?
 ├─ yes → focus
 └─ no
      ↓
detect capabilities
      ↓
Directory created
```

If repository:

```text
Open

New worktree for agent
```

---

# 51. Worktree Creation

Flow:

```text
New worktree for agent
       ↓
task/branch name
       ↓
GitService
       ↓
git worktree add
       ↓
DirectoryService
       ↓
new Directory
       ↓
AgentService
       ↓
OpenCode session
```

One user action should eventually create the complete task environment.

---

# 52. Activity Bus

Rust maintains normalized domain events.

Examples:

```text
directory.opened

terminal.started
terminal.exited

file.changed

git.changed

agent.started
agent.file_changed
agent.command_started
agent.command_finished
agent.waiting
agent.completed

review.created
review.sent

editor.opened
```

This decouples Git, terminal, OpenCode and Vue.

---

# 53. Status Bar

Left:

```text
agent/billing
worktree of ledger-api
```

Center:

```text
2 files changed
Review: 1/2 viewed
```

Right:

```text
agent: running tests
```

Global attention can override:

```text
billing: 2 drafts unsent
```

Avoid duplicating information already obvious elsewhere.

---

# 54. Responsive Layout

Wide:

```text
Sidebar | Sessions | Inspector
```

Medium:

```text
Sidebar | Sessions
            +
       Inspector overlay
```

Narrow:

```text
Sessions

Sidebar   → drawer
Inspector → drawer
```

Do NOT squeeze three columns until none are usable.

---

# 55. Missing Directory

If a directory/worktree disappears externally:

```text
billing
⚠ Missing
```

Actions:

```text
Locate
Close
```

Do not silently remove it.

---

# 56. Terminal Exit

When a process exits:

```text
zsh
Exited · code 1

[Restart]
[Close]
```

Keep its pane/history available until explicitly closed.

---

# 57. Security

Tauri's privileged Rust layer MUST expose narrow domain commands.

Do not give WebView code unrestricted:

```text
filesystem access
shell execution
process spawning
Git execution
```

Validate:

```text
directory IDs
canonical paths
command arguments
file paths
editor arguments
agent session ownership
```

Never concatenate shell commands from arbitrary UI strings.

Use argument arrays/process APIs.

---

# 58. Markdown Security

Markdown is untrusted content.

Requirements:

```text
sanitize HTML
restrict dangerous URLs
do not execute scripts
do not allow arbitrary WebView privileges
```

SVG requires equivalent care.

---

# 59. Performance Architecture

Never perform expensive work on the UI thread.

```text
Vue
 ↓
small typed requests
 ↓
Rust workers/tasks
 ↓
results
```

Streams:

```text
PTY
Agent
Process
```

use channels.

State events:

```text
Git changed
Agent status changed
Directory changed
```

can use lightweight application events/state updates.

---

# 60. Performance Targets

Initial engineering targets:

```text
Cold startup             < 2 s

Warm startup             < 1 s target

Terminal input latency   imperceptible

UI                        60 fps minimum target

Idle CPU                  approximately zero

Directory switching       < 100 ms perceived where cached

Typical Git status        < 300 ms

Memory                    bounded as terminals/sessions grow
```

Targets MUST eventually be backed by benchmarks.

---

# 61. Terminal Benchmarks

Before committing to a renderer:

Test:

```text
yes
large log streams
large cat
ripgrep output
npm/pnpm builds
cargo build
OpenCode
nvim
htop
rapid resize
multiple terminals
```

Measure:

```text
input latency
output throughput
CPU
RAM
frame time
IPC throughput
dropped bytes
scrollback behavior
resize latency
```

Compare at minimum:

```text
WebView terminal renderer
vs
libghostty prototype
```

---

# 62. Git Benchmarks

Test repositories:

```text
small repo
large monorepo
100k+ files
thousands of modified files
multiple worktrees
large binary files
large diffs
```

Measure:

```text
status
branch detection
worktree detection
single-file diff
full change discovery
watcher invalidation
```

---

# 63. MVP

v1 MUST support:

### Directory

- Open plain directory
- Open repository
- Detect worktree
- Restore directories
- Canonical-path identity

### Sidebar

- Directory hierarchy
- Sessions
- Agent attention state

### Terminal

- Real PTY
- Shell
- Multiple sessions
- Resize
- Scrollback
- Interactive TUI applications

### Files

- Tree
- File selection
- Git decoration
- Open externally

### Git

- Branch
- Status
- Changed files
- Diff
- Worktree detection

### Preview

- Markdown
- Images
- Text/code

### Review

- Line comments
- Draft comments
- Persist drafts
- Batch send to agent

### OpenCode

- Start session
- Observe state
- Observe activity
- Send message
- Send review
- Cancel
- Handle permissions
- Follow active file

### Editors

- Zed
- Neovim

### Persistence

- Directories
- Layout
- Settings
- Draft comments

---

# 64. P0 Technical Spikes

Do these before building most of the product.

## Spike 1 — Terminal

```text
Tauri
+
portable-pty
+
terminal renderer
+
Channel<ArrayBuffer>
```

Prove:

```text
interactive shell
nvim
OpenCode
resize
massive output
low latency
multiple terminals
```

---

## Spike 2 — OpenCode

Prove:

```text
start
connect
create session
subscribe
status
active file
tool execution
send message
cancel
permission handling
reconnect
```

---

## Spike 3 — Git

Prove:

```text
repository detection
canonical directory
worktree detection
parent repository
branch
status
diff
create worktree
deleted worktree
```

---

# 65. Development Order

```text
01  Tauri + Vue + TypeScript + Vite
02  shared domain model
03  Rust service architecture
04  DirectoryService
05  Git spike
06  PTY spike
07  TerminalService
08  Terminal UI
09  Sidebar
10  Session model
11  Files
12  Inspector
13  Git status
14  Diff viewer
15  Preview
16  SQLite persistence
17  EditorService
18  Zed
19  Neovim
20  OpenCode spike
21  AgentBridge
22  Agent UI
23  Follow Agent
24  Review comments
25  Send Review
26  Worktree creation
27  Multiple-agent attention
28  performance profiling
29  packaging
30  polish
```

---

# 66. First Vertical Slice

The first useful Marvis build is:

```text
Tauri
├── Vue UI
├── Directory sidebar
│
└── Rust Core
    ├── DirectoryService
    ├── GitService
    └── TerminalService
         ↓
     portable-pty
```

Acceptance test:

```text
1. Launch Marvis.

2. Open ~/dev/project.

3. Marvis detects that it is a Git repository.

4. Sidebar shows project + branch.

5. Create shell session.

6. Shell starts in ~/dev/project.

7. Run nvim.

8. Resize the Marvis window.

9. PTY resizes correctly.

10. Modify a file.

11. Filesystem watcher detects it.

12. Git status updates.

13. Inspector shows the changed file.

14. Select Changes.

15. Diff renders.
```

Do not build significant OpenCode UI before this works reliably.

---

# 67. Second Vertical Slice

Add the complete agent loop:

```text
Create worktree
      ↓
Open directory
      ↓
Start OpenCode
      ↓
Agent works
      ↓
Follow Agent
      ↓
Files change
      ↓
Git refresh
      ↓
Review diff
      ↓
Draft comments
      ↓
Send Review
      ↓
Agent continues
```

When this works, the central product hypothesis of Marvis is validated.

---

# 68. Definition of v1 Done

Marvis v1 is complete when a developer can:

```text
Open a repository

Create/open a worktree

Run several terminal sessions

Run interactive terminal applications

Launch OpenCode in that directory

Continue using Marvis while the agent works

See which agent requires attention

Follow files being modified

Review resulting Git changes

Write multiple review comments

Send them together to OpenCode

Watch the agent continue

Preview Markdown/images

Open exact files/lines in Zed or Neovim

Run two agents in separate worktrees

Restart Marvis without losing directory/layout/review state
```

---

# 69. Architecture Invariants

These rules MUST remain true as Marvis grows.

### Invariant 1

```text
Directory is the unit of context.
```

### Invariant 2

```text
Session belongs to Directory.
```

### Invariant 3

```text
Inspector derives from active Directory.
```

### Invariant 4

```text
Vue never directly controls system tools.
```

### Invariant 5

```text
Rust owns system integration.
```

### Invariant 6

```text
Git is the source of truth for Git.
```

### Invariant 7

```text
OpenCode is the source of truth for OpenCode.
```

### Invariant 8

```text
Terminal PTY and Terminal Renderer are separate.
```

### Invariant 9

```text
Agent implementation is hidden behind AgentBridge.
```

### Invariant 10

```text
Marvis reviews are local, agent-oriented review rounds.
```

---

# 70. Product Identity

**Name**

Marvis

**Product description**

> Marvis is a local developer workspace for running, supervising and reviewing development work across directories, Git worktrees, terminals and coding agents.

**Technical description**

> Marvis is a Tauri-based local control plane for agentic software development.

**Core object**

```text
Directory
```

not:

```text
Tab
Window
Chat
Project abstraction
```

**Core workflow**

```text
Directory
→ Agent
→ Code
→ Diff
→ Review
→ Agent
```

**Core philosophy**

> Orchestrate development tools instead of replacing them.
