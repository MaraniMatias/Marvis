# Marvis — Delivery Plan

> Companion to `MARVIS_REQUIREMENTS.md` (v0.2). This document does **not** reduce scope. It orders it, and it corrects the domain model.

**Status:** Draft v0.4
**Supersedes:** `MARVIS_DELIVERY_PLAN.md` v0.3 (the "worktree-first" model is withdrawn)
**Relationship to v0.2:** Everything in v0.2 remains in scope. This document replaces sections 8, 9, 10, 12, 39, 50, 51, 63, 65, 66, 67 and 68, and amends sections 57 and 60.

---

# 1. Scope Statement

Marvis will implement **everything** described in `MARVIS_REQUIREMENTS.md`. There is no MVP cut and no feature is dropped.

The work ships in **three deliveries**. Each leaves the developer with a tool they can use daily. A delivery is not scaffolding for the next one.

```text
Delivery 1   The Workstation     terminals, repos, checkouts, files, git, diff, editors
Delivery 2   The Agent Loop      OpenCode, follow, review, multi-agent attention
Delivery 3   Depth               reanchoring, rich previews, Neovim RPC, renderer, benchmarks
```

Ordering principle:

```text
Delivery 1 must be useful with no agent at all.
Delivery 2 must validate the central product hypothesis.
Delivery 3 must improve quality without changing architecture.
```

This document defines an **application**, not a workflow. It states what Marvis can do and what it guarantees. How a person organizes their work with those capabilities is their decision.

---

# 2. Domain Model

The model has three levels, each with a distinct role.

```text
Repo          A grouping. Identity plus a list of checkouts. Has no sessions of its own.
Checkout      A place where work happens. Has a branch, sessions, status and a diff.
Session       A process running in a checkout: shell, agent, nvim, server, custom.
```

```text
ledger-api                 main          ← Repo
├─ main                                  ← Checkout (primary)
│   └─ zsh                               ← Session
├─ billing                 listo         ← Checkout (worktree)
│   └─ opencode            ●
├─ migrate                 +118          ← Checkout (worktree)
│   ├─ opencode            ●
│   └─ psql
└─ docs-api                idle          ← Checkout (worktree)
```

## 2.1 Uniform checkouts

**Every checkout has the same capabilities.** Whether it is the original checkout of a repository or an additional one created with `git worktree add` makes no difference to what Marvis allows. Git treats them equally, and Marvis does not invent a permission hierarchy Git does not have.

The only differences between checkouts are data, not permissions:

```text
isPrimary      true for the original checkout; it cannot be removed with
               `git worktree remove`
branch         each checkout has its own
aheadOfDefault commits ahead of the repo's default branch
```

## 2.2 Types

```ts
type RepoKind = "git" | "plain";

interface Repo {
  id: string;
  kind: RepoKind;
  name: string;
  root: string;                 // canonical path of the primary checkout
  defaultBranch?: string;       // detected, read-only; absent for plain
  checkouts: Checkout[];        // primary first, then worktrees
  createdAt: string;
  lastOpenedAt: string;
}

interface Checkout {
  id: string;
  repoId: string;

  path: string;
  canonicalPath: string;

  isPrimary: boolean;
  branch?: string;              // undefined on detached HEAD or plain
  head?: string;

  aheadOfDefault?: number;
  changedFiles: number;

  sessions: Session[];
}
```

A plain folder is a `Repo` with `kind: "plain"` and exactly one checkout. This keeps the sidebar to a single structure with no special-case branching. A plain checkout has no branch, no diff and no review.

## 2.3 Identity

```text
Repo identity        canonical path of the primary checkout
Checkout identity    canonical path of the checkout
```

Opening a path that resolves to an already-known checkout **focuses it**. It never creates a duplicate.

## 2.4 Capability matrix

Capabilities are uniform across checkouts. They differ only by whether Git is present.

| Capability                       | Plain | Git checkout |
| -------------------------------- | :---: | :----------: |
| Terminal and sessions            |   ✓   |      ✓       |
| Files                            |   ✓   |      ✓       |
| Preview                          |   ✓   |      ✓       |
| Zed / Neovim                     |   ✓   |      ✓       |
| Branch and status                |       |      ✓       |
| Diff                             |       |      ✓       |
| Review                           |       |      ✓       |
| Agent                            |       |      ✓       |
| Create worktree from this repo   |       |      ✓       |

Agent and Review require Git because both depend on a diff.

## 2.5 The diff

**The diff is always taken against the repo's default branch.** There is no selector and no per-checkout reference.

```text
git diff <default>...HEAD    from the merge-base to HEAD
+ uncommitted changes on top
```

The three-dot form shows only what this checkout did since it diverged. It does not show commits that landed on the default branch afterwards, so several agents merging in parallel do not pollute each other's reviews.

The default branch is **detected**, not configured:

```text
1  git symbolic-ref refs/remotes/origin/HEAD
2  the branch the repository was initialized with
3  if neither resolves, ask the user once and remember the answer
```

The diff header always names it, for example `vs main`.

Because the reference is a constant of the repo, Marvis stores no diff reference per checkout.

**Commits ahead.** The status bar shows how many commits the checkout is ahead of the default branch. If a checkout was created from a different branch outside Marvis, its diff will include commits that are not its own. That is correct under this rule, and the commit count makes the size of the diff unsurprising.

## 2.6 Guarantee: concurrent activity is visible

Marvis does not prevent two actors from operating on the same checkout. It guarantees the user can see it.

```text
Actors     a terminal with a running process, an editor, an agent
Signal     when more than one actor has activity in the same checkout, the
           sidebar and the status bar show it
```

The signal is informational. It does not block and does not lecture.

## 2.7 Constraint that comes from Git

A branch cannot be checked out in two checkouts at once. Git rejects it. Marvis surfaces this as a clear message when creating a worktree or switching branch, naming the checkout that already holds the branch, instead of passing through the raw Git error.

## 2.8 Resolving the hierarchy on open

A checkout does not exist without its repo. Opening a path always resolves the repo first.

```text
Path opened                            Result
─────────────────────────────────────  ─────────────────────────────────────────
Primary checkout                       Repo appears with its existing worktrees
A worktree, repo not yet open          Parent repo is resolved and registered;
                                       ALL worktrees of the repo are listed;
                                       the opened one is focused
A subdirectory inside a checkout       Resolves to the containing checkout,
                                       which is focused; no new node is created
A path reached through a symlink       Canonicalized first, then matched
Two worktrees of the same repo         One repo, two checkouts
A folder with no Git                   Plain repo, one checkout
```

When a worktree is opened on its own, Marvis lists **all** worktrees of the repo (`git worktree list --porcelain`), not only the one opened. Showing a partial list would show a structure that does not exist. Worktrees the user did not explicitly open appear collapsed, with no sessions, but visible.

Resolution uses Git as the source of truth (`git rev-parse`, `git worktree list --porcelain`). Marvis does not rely solely on parsing `.git`.

---

# 3. Sidebar

The sidebar represents **places**. It renders the three levels of section 2.

## 3.1 Structure

```text
▾ ledger-api                 main              Repo
  ├─ ▾ main                                    Checkout (primary)
  │     zsh
  ├─ ▾ billing               listo             Checkout
  │     opencode             ●                 Session
  ├─ ▾ migrate               +118
  │     opencode             ●
  │     psql
  └─ ▸ docs-api              idle
```

## 3.2 Two independent signals

The sidebar carries two kinds of signal, and they must not be conflated.

```text
Checkout label    (right of the checkout)   summary: changes, state, idle
Session dot       (right of the session)    does this need the user
```

## 3.3 Checkout labels

| Label         | Meaning                                                                 |
| ------------- | ----------------------------------------------------------------------- |
| `+N`          | N changed lines or files against the default branch                     |
| `listo`       | An agent finished and its changes are not yet reviewed (Delivery 2)     |
| `idle`        | No active session and no pending changes; requires nothing from the user |
| `Missing`     | The directory no longer exists on disk                                  |

`idle` and `listo` are different by rule, not by styling. `idle` requires nothing. `listo` requires the user.

## 3.4 Session dots

```text
green   running and healthy (e.g. a dev server)
blue    agent actively working
amber   the user is needed: agent finished, permission requested,
        waiting for input, or unsent review drafts
red     error or non-zero exit
none    inactive
```

## 3.5 Primary checkout node

The primary checkout is always a node, sibling to the worktrees, and it is where sessions in the repo root live. If it has no sessions, it renders collapsed and discreet but is never omitted, because it is the place where a terminal in the repo root is opened.

## 3.6 Interactions

```text
Click a repo             expand / collapse
Click a checkout         activate it; expand its sessions
Click a session          focus it in the main area
Drag a session           reorder within its checkout
Context menu             rename, close, duplicate, reveal in Finder, copy path
Bottom action            Open directory (⌘K); New worktree when a repo has focus
```

Closing a session that has a running process asks for confirmation.

## 3.7 Attention inbox

The sidebar doubles as an attention inbox. A checkout in the background that needs the user stays visible and does not collapse away.

---

# 4. Worktree Lifecycle

Creating and removing worktrees is a capability of the app, available from any Git checkout of a repo.

```text
create      git worktree add
open        register as Checkout of its Repo
work        sessions, agent, changes
review      diff against the default branch
remove      git worktree remove, with safeguards
prune       git worktree prune for stale entries
```

## 4.1 Creation

```text
New worktree
   ↓
task name → branch name (editable)
   ↓
git worktree add   (new branch created from the default branch)
   ↓
register as Checkout
   ↓
open and focus
   ↓
Delivery 2: start an OpenCode session in it
```

New worktrees are created from the default branch. This is a rule of the app, consistent with the diff rule in section 2.5.

Default location: `<repo>/.worktrees/<name>`, configurable. See Open Question Q1 for how `.worktrees/` avoids appearing as untracked content in the primary checkout.

## 4.2 Removal safeguards

```text
Uncommitted changes    require explicit confirmation, offer commit or stash
Unmerged commits       warn and show the commit count
Active sessions        list them and require confirmation
Active agent           block until the agent is cancelled or completed
Primary checkout       cannot be removed as a worktree
```

Integration (merge, rebase, pull request) is out of scope for v1 and is done from a shell session, consistent with v0.2 section 4.

---

# 5. Delivery 1 — The Workstation

## 5.1 Goal

A developer can open a repository, see and create checkouts, run several terminals in them, browse files, see live Git status, read diffs, and jump into Zed or Neovim. No agent integration is required for this delivery to be useful.

## 5.2 Exit statement

> I can work in Marvis all day across several checkouts, using my shell, Neovim and Zed, and I never leave it to see what changed.

## 5.3 Scope

### Application shell

```text
Tauri 2 + Vue 3 + TypeScript + Vite
shadcn-vue + Reka UI + Tailwind CSS
Rust core with the service architecture from v0.2 section 46
Vue structure from v0.2 section 47
Tauri IPC: Commands and Channels (v0.2 section 7)
Activity bus (v0.2 section 52), with the events needed by this delivery
```

### Repo and checkout

```text
Open a plain folder, a primary checkout, or a worktree
Resolve the hierarchy on open (section 2.8)
Detect the default branch (section 2.5)
Canonical-path identity, focus on duplicate open
List all worktrees of a repo
Worktree creation (section 4.1)
Worktree removal with safeguards (section 4.2)
Branch-in-use message (section 2.7)
Concurrent-activity signal (section 2.6)
Missing directory handling (v0.2 section 55)
Restore repos and checkouts on launch
```

### Sidebar

```text
Three-level hierarchy (section 3)
Checkout labels and session dots (sections 3.3, 3.4)
Attention states green, red, none active in this delivery
```

Blue and amber are rendered in Delivery 2. The indicator system is built now so it does not need reworking.

### Terminal

```text
Real PTY through portable-pty
TerminalBackend trait (v0.2 section 17)
Renderer behind an adapter, WebView renderer first (v0.2 section 19)
Multiple sessions per checkout
Tabs and horizontal/vertical splits, including nested splits
Resize with correct PTY propagation
Scrollback
Mouse, clipboard, alternate screen, bracketed paste, true color, Unicode
Interactive TUI applications: nvim, vim, htop
Terminal exit handling (v0.2 section 56)
Shell environment inheritance
```

Session types active in this delivery: `shell`, `nvim`, `server`, `custom`. The `agent` type is reserved.

### Files

```text
Directory tree of the active checkout
.gitignore-aware behavior
Git decorations (modified, added, untracked)
Virtualized rendering
Fuzzy file search
File selection
Open in editor
```

### Git

```text
Git CLI through GitService, argument arrays only
Repo, checkout and default-branch detection
Branch and HEAD
git status --porcelain=v2
Changed files and commits ahead of the default branch
Filesystem watcher: debounced, coalesced, scoped per checkout
Diff against the default branch, merge-base form (section 2.5)
File-by-file diff loading
Unified diff, collapsible hunks, syntax highlighting, line numbers
Viewed state per file
Large diff protection: lazy load, virtualization, file-level limits, binary detection
```

### Inspector

```text
Files, Changes, Preview tabs
Tabs render only when capabilities allow (section 2.4)
The inspector always derives from the active checkout
```

### Preview

```text
Markdown: CommonMark/GFM, tables, task lists, code blocks with highlighting,
          relative images, sanitized HTML
Plain text and source code
```

### Editors

```text
EditorAdapter trait (v0.2 section 40)
ZedAdapter: open directory, file, file:line, file:line:column
NeovimAdapter: launch in a Marvis session with +line
Detect availability of each editor
```

### Persistence

```text
SQLite
Repos, checkouts and their order
Session metadata
Layout and splits
Preferences
Recent paths
Window geometry
Inspector state
```

Marvis stores no per-checkout diff reference (section 2.5).

### Command palette

```text
⌘K
Open Directory, New Worktree, New Terminal, Open File,
Open Changes, Open Preview, Open in Zed, Open in Neovim
```

### Status bar

```text
Branch, commits ahead of the default branch, changed-file count,
viewed progress, concurrent-activity signal
```

### Security

```text
Narrow domain commands only, no generic shell exposure to the WebView
Canonical path validation
Argument arrays, no string-concatenated commands
Markdown sanitization
```

## 5.4 Required spikes before Delivery 1 construction

### Spike 1 — Terminal pipeline

```text
Question   Does Tauri Channel<ArrayBuffer> + portable-pty + a WebView renderer
           hold up under real workloads?
Test       yes, large cat, ripgrep over a large tree, cargo build, pnpm install,
           nvim, htop, rapid resize, five terminals at once
Measure    input latency, output throughput, CPU, RAM, frame time,
           dropped bytes, resize latency
Pass       no dropped bytes, no UI freeze under `yes`, nvim fully usable,
           idle CPU near zero
Fail path  evaluate an alternative renderer or a chunking strategy before proceeding
```

### Spike 3 — Git, repos and checkouts

```text
Question   Can Git alone reliably provide the repo/checkout structure,
           the default branch and status?
Test       open a worktree with its repo closed; open a subdirectory of a
           checkout; open through a symlink; worktree inside .worktrees/;
           deleted worktree directory; detached HEAD; repo with no commits;
           repo with no remote (default branch detection);
           branch already checked out elsewhere; submodules;
           status time on large repositories
Pass       correct repo, checkout and default branch in every case;
           the hierarchy of section 2.8 resolves without user input;
           status within the budgets of section 9.2
```

Numbering follows v0.2 section 64. Spike 2 (OpenCode) belongs to Delivery 2.

## 5.5 Build order

```text
01  Tauri + Vue + TypeScript + Vite scaffold
02  Shared domain model: Repo, Checkout, Session
03  Rust service architecture and error model
04  SQLite persistence and migrations
05  RepoService and CheckoutService, canonical identity
06  Spike 3: Git
07  GitService: detection, default branch, status, worktree list
08  Hierarchy resolution on open (section 2.8)
09  Worktree creation from the default branch
10  Spike 1: Terminal
11  TerminalService + TerminalBackend + renderer adapter
12  Terminal UI, tabs, nested splits, resize
13  Sidebar: three levels, labels, dots
14  Session model, activeCheckout vs activeSession
15  FileService, watcher, debounce/coalesce
16  Files tab, virtualization, Git decorations
17  Inspector shell and capability-driven tabs
18  Diff viewer: merge-base form, file-by-file, large diff protection
19  Preview: Markdown, text, source
20  EditorService, Zed adapter, Neovim launch adapter
21  Command palette
22  Worktree removal with safeguards
23  Concurrent-activity signal
24  Missing directory and terminal exit handling
25  Status bar
26  Persistence of layout and restore
27  Hardening, security pass, packaging
```

## 5.6 Acceptance criteria

```text
D1-01  Open a primary checkout; its existing worktrees appear as sibling
       checkouts under the repo.
D1-02  Open a worktree whose repo is not open; the repo is resolved
       automatically, all its worktrees are listed, and the opened one is
       focused.
D1-03  Open a subdirectory of a checkout; the containing checkout is focused
       and no new node is created.
D1-04  Open the same path twice, including through a symlink; the existing
       checkout is focused, not duplicated.
D1-05  Create a worktree from the palette; it is created from the default
       branch, opens as a checkout, and its shell starts inside it.
D1-06  Try to create a worktree on a branch already checked out elsewhere;
       Marvis names the checkout that holds it.
D1-07  Run nvim in a session and resize the window; nvim redraws correctly.
D1-08  Run `yes` for 30 seconds; the UI stays responsive and no bytes are dropped.
D1-09  Modify a file from a terminal; Git status and the Changes tab update
       without a manual refresh.
D1-10  The diff header names the default branch and shows the merge-base
       diff; commits landed on the default branch afterwards do not appear.
D1-11  Open a 4,000+ line diff; the app does not freeze and only visible
       hunks are rendered.
D1-12  Select a Markdown file; it renders with tables and sanitized HTML.
D1-13  Open a file at an exact line in Zed and in Neovim.
D1-14  Delete a worktree directory externally; the checkout shows Missing and
       offers Locate and Close.
D1-15  Remove a worktree with uncommitted changes; Marvis refuses without
       explicit confirmation. The primary checkout cannot be removed.
D1-16  Run a terminal process and edit a file in an editor within the same
       checkout; the concurrent-activity signal appears.
D1-17  Quit and relaunch; repos, checkouts, layout and splits are restored.
D1-18  Idle CPU is near zero with five open sessions.
D1-19  A repo with no remote and no `main` resolves its default branch or asks
       once; it never shows a wrong reference silently.
D1-20  The WebView cannot execute arbitrary commands; only domain commands
       are reachable.
```

---

# 6. Delivery 2 — The Agent Loop

## 6.1 Goal

A developer can launch OpenCode in a checkout, supervise it, review its changes with line comments, send the review back as one message, and run several agents in parallel with clear attention signals.

This delivery validates the central hypothesis of the product.

## 6.2 Exit statement

> I can run two agents in two checkouts, know at a glance which one needs me, review what it did, send my feedback in one action, and watch it continue.

## 6.3 Scope

### Agent architecture

```text
AgentBridge trait (v0.2 section 35)
OpenCodeAgentBridge as first implementation
Domain naming: Agent, AgentSession, AgentStatus, AgentEvent
Normalized AgentEvent stream (v0.2 section 37)
Internal states: starting, thinking, working, running_command,
                 waiting_permission, waiting_input, waiting_review,
                 completed, failed
UI collapse: working, needs you, done, error
OpenCode remains the source of truth for agent state (v0.2 invariant 7)
```

### Agent lifecycle

```text
Start an OpenCode session in a Git checkout
Observe state and activity
Observe the active file
Observe command execution
Send a message
Cancel
Respond to permission requests
Reconnect after a dropped connection
Handle OpenCode not installed or not responding
```

Agent sessions require a Git checkout (section 2.4).

### Worktree-to-agent flow

Completes the flow started in Delivery 1:

```text
New worktree → open checkout → start OpenCode → agent works
```

One user action creates the whole task environment.

### Follow Agent

```text
Follow Agent toggle in the inspector footer
agent edits file → file.active event → inspector selects the file
Manual selection pauses Follow, showing "Follow paused · Resume"
Follow defaults to OFF when a review begins
```

### Review

```text
Line comments on the diff
Range comments
Draft state, persisted in SQLite
Edit and delete drafts
Batch send: all drafts as one message to the agent
Status: draft → sent → resolved
Review rounds
Viewed progress feeds the status bar
Target session selection when a checkout has more than one agent session
```

Anchoring in this delivery stores file, side, line/range and content hash. Automatic **reanchoring** is Delivery 3. Until then, a comment whose target changed is marked `outdated`. It is never silently attached elsewhere (v0.2 section 33).

### Review completion signal

Addresses a gap in v0.2. Marvis marks a comment `sent`, but only the agent knows whether it acted on it.

```text
After a review is sent and the agent finishes its next turn:
  compute the new diff
  for each sent comment, show whether its lines changed
  offer one-click "Mark resolved" per comment
```

This closes the loop without Marvis needing to interpret the agent's intent.

### Multiple agents

```text
Attention states fully active: blue, amber, red
The sidebar behaves as an attention inbox (section 3.7)
Global status bar override, e.g. "billing: 2 drafts unsent"
Background checkouts needing attention remain visible
```

### Sending while the agent is working

Decision required (Q4). Proposed default:

```text
Warn that the agent is mid-task and offer:
  Send now    interrupts context
  Queue       sent when the agent finishes its turn
  Cancel
```

### Session type and palette

```text
SessionType "agent" becomes active
New OpenCode Session, Send Review, Toggle Follow Agent, Cancel Agent
```

## 6.4 Required spike before Delivery 2 construction

### Spike 2 — OpenCode

```text
Question   Can Marvis reliably observe and control OpenCode through a stable
           interface?
Test       start, connect, create session, subscribe to events, read status,
           detect active file, detect tool execution, send message, cancel,
           handle permission requests, reconnect after disconnect
Pass       every item works without scraping the PTY
Fail path  if events are not exposed, fall back to PTY heuristics for status
           only, and mark those features best-effort
```

The integration channel (HTTP server, SDK, or process I/O) must be confirmed against OpenCode's current documentation. It remains a hypothesis until this spike resolves it.

Cancel and permission handling depend on an interface Marvis does not control. If the spike cannot demonstrate them cleanly, they ship as best-effort with the limitation documented, and are hardened once the interface is understood.

## 6.5 Build order

```text
28  Spike 2: OpenCode
29  AgentBridge trait and OpenCodeAgentBridge
30  AgentService, event normalization, activity bus events
31  Agent session type in the session model
32  Start agent from the worktree creation flow
33  Agent UI: status, activity, message input
34  Attention states in sidebar and status bar
35  Follow Agent
36  Review: comment creation on diff, draft persistence
37  Review: batch send, target selection, review rounds
38  Cancel and permission handling
39  Sent-comment resolution signal
40  Send-while-working policy
41  Multi-agent attention
42  Reconnect and failure handling
43  Hardening and packaging
```

## 6.6 Acceptance criteria

```text
D2-01  From any Git checkout, one action creates a worktree, opens it, and
       starts OpenCode inside it.
D2-02  Agent status is visible in the sidebar for a checkout that is not
       currently focused.
D2-03  With Follow Agent on, the inspector tracks the file being edited; a
       manual selection pauses it and shows "Follow paused".
D2-04  Add three line comments across two files; they persist across restart
       as drafts.
D2-05  "Send review" delivers all drafts to the agent as a single message.
D2-06  Two agents run in two checkouts; the one that finishes first shows
       amber while the other stays blue.
D2-07  A permission request from the agent appears in Marvis and can be
       answered from it.
D2-08  Cancel stops the agent; partial changes remain visible in the diff.
D2-09  After a sent review and a new agent turn, each comment shows whether
       its lines changed and can be marked resolved.
D2-10  A comment whose line changed is marked outdated and is never attached
       to unrelated code.
D2-11  Killing the OpenCode process shows an error state and offers restart;
       drafts are not lost.
D2-12  With OpenCode missing from PATH, the app explains it and remains usable.
D2-13  Restart Marvis with two agents active and unsent drafts; repos,
       layout and drafts are restored.
D2-14  An agent and a terminal editing the same checkout trigger the
       concurrent-activity signal.
D2-15  The full loop completes end to end: worktree → agent → diff → review →
       send → agent continues → review again.
```

---

# 7. Delivery 3 — Depth

## 7.1 Goal

Raise quality and cover the remaining planned capabilities without changing the architecture established in Deliveries 1 and 2.

## 7.2 Exit statement

> The tool no longer has rough edges in my daily use, and the deferred capabilities are in place.

## 7.3 Scope

### Comment reanchoring

Amends v0.2 section 33 from "mark outdated" to automatic recovery.

```text
Persist file, side, line/range, nearby context lines, content hash
After a change: attempt reanchor by content match within the file
High confidence   reanchor with a visible marker
Low confidence    mark the comment outdated
Never attach a comment to unrelated code
Show original and current position when reanchored
```

The matching strategy and threshold are open (Q5). They should be tuned against real agent-modified diffs collected during Delivery 2, which is one reason this work follows it.

### Rich preview

```text
Images: PNG, JPEG, WebP, GIF
        fit, 100%, zoom, dimensions, open externally
        large images do not block the UI thread
SVG:    rendered without script execution
JSON:   formatted, collapsible
YAML:   formatted
```

All new preview types follow v0.2 section 58.

### Neovim RPC integration

```text
Marvis → Neovim over a socket → an existing Neovim instance
Open a file at a line in an already-running Neovim
Reuse an existing Neovim instance in the checkout instead of spawning another
Detect the socket; handle a missing or stale one
```

Bidirectional integration (current buffer, cursor, selection) and a `marvis.nvim` plugin remain optional and are not required by this delivery.

### Terminal renderer evaluation

```text
libghostty spike per v0.2 section 20
Compare against the WebView renderer using the benchmarks in section 9.1
Adopt only if the gain justifies the native complexity
Marvis remains fully functional without it
```

### Layout and ergonomics

```text
Responsive layout modes (v0.2 section 54)
  wide      three columns
  medium    inspector as overlay
  narrow    sidebar and inspector as drawers
Keyboard accessibility across all panels
Configurable font size for terminal and inspector
Light and dark themes following the system
```

### Diff enhancements

```text
Side-by-side diff
Stage / unstage hunk
```

### Performance program and polish

```text
Benchmark suites, profiling, regression tracking
Memory bounds as sessions grow
Empty states and error messages review
First-launch onboarding
Packaging, signing and update path
```

## 7.4 Build order

```text
44  Reanchoring engine and tuning
45  Image preview
46  SVG preview with script-safe rendering
47  JSON and YAML preview
48  Neovim RPC adapter
49  libghostty spike and comparison
50  Side-by-side diff
51  Stage and unstage hunk
52  Responsive layout
53  Accessibility pass
54  Themes and font settings
55  Benchmark suites and regression tracking
56  Polish, onboarding, packaging
```

## 7.5 Acceptance criteria

```text
D3-01  After the agent moves commented code, the comment follows it when
       confidence is high and is marked outdated when it is not; it is never
       attached to unrelated code.
D3-02  PNG, JPEG, WebP and GIF preview with zoom; a large image does not
       freeze the UI.
D3-03  An SVG containing a script renders without executing it.
D3-04  JSON and YAML files render formatted in Preview.
D3-05  Opening a file from Marvis reuses a running Neovim in that checkout and
       jumps to the exact line.
D3-06  A stale or missing Neovim socket falls back to launching a new
       instance with a clear message.
D3-07  A documented benchmark report compares the WebView renderer with the
       libghostty prototype, and the decision is recorded.
D3-08  Resizing to a narrow window keeps sessions usable, with sidebar and
       inspector as drawers.
D3-09  Every panel is reachable and operable by keyboard alone.
D3-10  A benchmark run produces stored results and flags regressions.
```

---

# 8. Architecture Invariants

v0.2 section 69 stands, with these changes.

```text
Invariant 1    (revised)  Repo groups; Checkout is the unit of context.
Invariant 2    (revised)  Session belongs to Checkout.
Invariant 3    (revised)  Inspector derives from the active Checkout.
Invariant 11   (new)      Every checkout has the same capabilities. Marvis makes
                          concurrent activity on one checkout visible.
Invariant 12   (new)      The diff always compares against the repo's default
                          branch, from the merge-base.
Invariant 13   (new)      A checkout never exists without its repo. Opening a
                          path resolves the repo first.
```

Invariants 4 to 10 are unchanged.

---

# 9. Performance and Benchmarks

Amends v0.2 sections 60 to 62.

## 9.1 Terminal benchmarks

Unchanged from v0.2 section 61. Run first in Spike 1 and again in Delivery 3 for the renderer comparison.

## 9.2 Git benchmarks with size-based budgets

v0.2 section 60 set one target: typical Git status under 300 ms. That figure cannot hold for every repository. It is replaced by a budget per repository size.

| Repository class | Files        | Status target                                   |
| ---------------- | ------------ | ----------------------------------------------- |
| Small            | < 5k         | < 100 ms                                        |
| Medium           | 5k – 50k     | < 300 ms                                        |
| Large monorepo   | 50k – 200k   | < 1 s, never blocking the UI                    |
| Very large       | > 200k       | Best effort, always asynchronous, never blocking |

The invariant across all classes is that Git work never blocks the UI thread. The numeric targets are starting points to be revised by Spike 3 measurements.

## 9.3 Global targets

```text
Cold startup             < 2 s
Warm startup             < 1 s target
Terminal input latency   imperceptible
UI                       60 fps minimum target
Idle CPU                 approximately zero
Checkout switching       < 100 ms perceived where cached
Memory                   bounded as terminals and sessions grow
```

---

# 10. Security Acceptance Tests

v0.2 section 57 states security as principles. They are converted into checks that can fail. Each is automated and runs in CI.

```text
SEC-01  A path containing `..` segments is canonicalized before use.
SEC-02  A symlink pointing outside the opened checkout is resolved, and file
        operations are rejected if they escape it.
SEC-03  A filename containing spaces, quotes, semicolons, `$()` and newlines
        is passed to git, editors and shells as a single argument and is
        never interpreted.
SEC-04  A Markdown file containing `<script>`, `onerror=` handlers and
        `javascript:` links renders with all of them neutralized.
SEC-05  The WebView has no capability to spawn an arbitrary process or read
        an arbitrary path; only domain commands are reachable.
SEC-06  A repo, checkout or session ID that does not belong to the caller is
        rejected.
SEC-07  A worktree or branch name containing shell metacharacters is handled
        as data.
```

---

# 11. Amendments Summary

Sections of `MARVIS_REQUIREMENTS.md` v0.2 affected by this plan.

| v0.2 section              | Change                                                                 |
| ------------------------- | ---------------------------------------------------------------------- |
| 8 Core Domain Model       | `Directory` replaced by `Repo` and `Checkout`                          |
| 9 Git Context             | Folded into `Checkout`; `defaultBranch` on `Repo`; no per-checkout base |
| 10 Capability Matrix      | Uniform across checkouts; only Git presence differentiates             |
| 12 Sidebar                | Three levels: repo, checkout, session                                  |
| 39 Multiple Agents        | Attention state attaches to a checkout                                 |
| 50 Opening a Directory    | Hierarchy resolves on open (section 2.8)                               |
| 51 Worktree Creation      | Available from any Git checkout; created from the default branch       |
| 57 Security               | Converted into automated tests (section 10)                            |
| 60 Performance Targets    | Git status target replaced by size-based budgets                       |
| 63 MVP                    | Replaced by three deliveries                                           |
| 65 Development Order      | Replaced by per-delivery build orders (steps 01–56)                    |
| 66, 67 Vertical Slices    | Absorbed into Delivery 1 and Delivery 2 acceptance criteria            |
| 68 Definition of v1 Done  | Replaced by per-delivery exit statements and criteria                  |
| 69 Invariants             | 1–3 revised; 11–13 added                                               |

Everything else in v0.2 stands as written.

---

# 12. Open Questions

Ordered by how much they affect architecture.

```text
Q1   Worktree location. `<repo>/.worktrees/<name>` keeps worktrees together
     but must not appear as untracked content in the primary checkout.
     Options: add the folder to .git/info/exclude automatically, or place
     worktrees outside the repo (e.g. ~/dev/<repo>.worktrees/). Decide
     before Spike 3, because path detection depends on the convention.

Q2   OpenCode integration channel: HTTP server, SDK, or process I/O. Must be
     confirmed against OpenCode's current documentation. Decides how much
     agent state is reliable. Resolved in Spike 2.

Q3   Where review comments are stored: inside the repo or in Marvis's own data
     directory. Comments belong to a checkout, so removing a worktree must
     either delete or archive its comments. Inclination: Marvis data
     directory, keyed by checkout id.

Q4   Sending a review while the agent is mid-task: interrupt, queue, or warn.
     Section 6.3 proposes warn-and-choose as the default.

Q5   Reanchoring strategy and confidence threshold. Tune with real diffs
     collected during Delivery 2.

Q6   Removing a worktree: what happens to its branch. Delete, keep, or ask.
     Inclination: ask, defaulting to keep.

Q7   Submodules inside a checkout: shown, ignored, or treated as directories.
     Surfaced by Spike 3.

Q8   Bare repositories. Probably unsupported at first; confirm in Spike 3.

Q9   macOS only at first (v0.2). Decide at the end of Delivery 1 whether to
     validate the Tauri + portable-pty stack on Linux before Delivery 2, to
     avoid macOS-specific assumptions hardening.

Q10  Cancelling an agent: what guarantee Marvis gives about partial changes.
     Inclination: none beyond making them visible in the diff.

Q11  Repos with no remote and no recognizable default branch. Section 2.5 asks
     the user once and remembers. Confirm that this is acceptable, or define
     a stricter fallback.

Q12  What constitutes "activity" for the concurrent-activity signal (section
     2.6): a running foreground process, an open editor, file writes in the
     last N seconds, or a combination. Needs a definition before it is built.
```
