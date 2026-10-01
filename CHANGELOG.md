# Changelog

One section per released version, newest first. The release workflow prepends the section matching
the version it is publishing to the notes GitHub generates from the commits, so a section here is
the part a reader should read and the generated list is the part they can skip. Keep the newest work
under `Unreleased` and rename that heading to the version when you tag it; a version with no section
here still gets its generated notes.

## Unreleased

- **`exit` closes the terminal it was typed in.** Ending a shell with `exit` or an alias of it like
  `:q` now takes the pane with it, instead of leaving a frozen last frame that reads as a hung app
  and an entry in the sidebar that looks alive. Closing a terminal whose process is still running
  still asks first.

- **The terminal and editor share a bundled typeface.** Fira Code Nerd Font Mono is no longer
  dependent on fonts installed on the machine; both surfaces use the same regular and bold faces.
  The terminal waits for them before opening xterm, so the initial grid and glyph atlas are created
  with the bundled font. This also addresses the reported selection-rendering inconsistency; visual
  verification on the affected macOS setup is still needed.

- **The terminal scrollbar is a choice.** It is hidden by default, which is what the terminal has
  always done; `Auto` shows it while you scroll and takes it away when you stop, and `Always` keeps
  it there to be dragged. All three are under the terminal breadcrumb. The thumb is drawn over the
  right edge rather than beside it, so it does not cost the grid a column.

- **A worktree somebody else created shows up in the panel.** More than one hand adds a worktree:
  Marvis's own dialog, an agent running `git worktree add` in a terminal, a script. Until now only
  the first was noticed while the app was open — a worktree added any other way appeared at the
  next launch, and its row was never watched once it did, so the numbers on it did not refresh.
  Marvis now watches the list of worktrees in the Git directory they share, so a worktree joining
  or leaving a repository is read again and the panel agrees with the disk. An archived worktree
  stays archived, and a worktree removed elsewhere becomes a row that says its directory is gone,
  which is what closing it from the panel has always done.

## 0.6.0

- **Markdown export is now the default review destination.** With no saved choice, sending review
  notes writes a non-overwriting Markdown file under `~/.marvis/tmp/code-reviews/` and opens it in
  Marvis for editing. This changes what existing users get from the review button: choose OpenCode
  once per checkout to keep sending rounds to an OpenCode session. Exporting does not create a
  round or mark notes sent. Saved open documents without an origin are dropped and the checkout
  view falls back to its default; review notes are unaffected.

## 0.3.0

- **A menu bar with the menus that earn their place.** Marvis used to open with the menu macOS
  gives every app that does not bring its own: a File, a View, a Window and a Help with nothing in
  it. What is left is the Marvis menu — About, Services, Hide, Hide Others, Quit — and Edit, which
  is not there by convention. macOS does not deliver `⌘C`, `⌘V`, `⌘X`, `⌘A` or `⌘Z` to the app at
  all: the Edit menu is what turns them into `copy:` and `paste:` and sends them into the window.
  Take it away and copying stops working in the search field, the editor and the terminal. Window
  went with the rest, so the traffic lights are how the window is minimized, zoomed and closed.

- **Archiving a worktree instead of losing it.** The cross on a worktree row now archives it: the
  row leaves the sidebar and nothing on disk moves — the branch, its commits and its files stay
  exactly where they are. The repo root grows a restore action that brings back every worktree
  that repository archived, in one click. Both actions ask first, in a dialog drawn like the rest
  of the app.

## 0.1.0

The first build anyone else could install: a `.dmg` for Apple Silicon and a `.deb` and `.AppImage`
for x86_64 Linux, published as a GitHub release.

- **One window for a repository.** The shell is a titlebar of breadcrumbs over a workdir sidebar, a
  details panel and a single main pane that switches between terminal, document and diff.
- **One agent per checkout.** The review loop runs end to end, each checkout gets its own bridge,
  and an agent mid-turn is asked before it is interrupted.
- **Documents and diffs.** Syntax highlighting with Shiki, the language taken from the file itself,
  front matter parsed once and read as YAML, and review notes and agent delivery inside the diff.
- **Terminals that earn their row.** One per checkout, the program in front of it named in the
  sidebar, nameable sessions, and a grid that keeps its columns inside the pane.
- **Losing nothing quietly.** One schema with no migrations, dead sessions no longer hoarded, and
  the four ways this app could drop work or a database closed.

## 0.2.0

- **Linux arm64.** `.deb` and `.AppImage` built on a native arm64 runner, so Raspberry Pi and ARM
  servers get a real binary instead of an emulated one.
- **macOS signed ad-hoc.** The bundle carries an ad-hoc signature with the hardened runtime, which is
  what keeps macOS from reporting a download from a release as damaged. It is not a Developer ID, so
  Gatekeeper still asks once.
- **Releases gated on their own commit.** A tag now runs the full CI against the tagged SHA before
  anything is built, the release lands as a draft with a `SHA256SUMS` beside the artifacts, and it is
  published only once every platform passed. The version is bumped, committed and tagged by
  `pnpm release`.
- **Builds are cached.** Neither workflow cached anything, so a release compiled the Rust graph
  three times over.
