# Changelog

One section per released version, newest first. The release workflow prepends the section matching
the version it is publishing to the notes GitHub generates from the commits, so a section here is
the part a reader should read and the generated list is the part they can skip. Keep the newest work
under `Unreleased` and rename that heading to the version when you tag it; a version with no section
here still gets its generated notes.

## Unreleased

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
