# Phase 1 — Markdown export as a review destination

Status: **PARTIAL** — the Markdown export path is implemented. The available destinations are
manual Markdown handoff and the existing OpenCode integration; Codex, Claude Code, and pi
integrations are not implemented. Export support does not complete the broader multi-agent
roadmap. Those adapters are separate follow-on work, not Phase 1 requirements.

This document records the current contract and implementation, not an implementation plan.

## Scope and status

| Requirement / capability                                                      | Status                                                                                                                     |
| ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| Choose Markdown or OpenCode per checkout; default to Markdown                 | Implemented                                                                                                                |
| Build review Markdown, write a unique file, and open it in Marvis             | Implemented                                                                                                                |
| Read, edit, copy the path of, and persist an open exported document           | Implemented                                                                                                                |
| Send a review to an OpenCode session with round tracking                      | Implemented; separate from export                                                                                          |
| Send directly to Codex, Claude Code, or pi                                    | Not implemented; specified in [Phase 2](todo/phase-2-codex.md), [Phase 3](todo/phase-3-claude-code.md), and [Phase 4](todo/phase-4-pi.md) |
| Browse old exports from the checkout file tree or automatically clean them up | Not implemented; outside Phase 1                                                                                           |

Markdown is a manual handoff: it writes a file for the user to inspect, edit, or paste elsewhere.
It does not send to an agent or prove that an agent received anything.

## Implemented destination and Markdown contract

The review destination type is `"markdown" | "opencode"` in
[`src/domain/review.ts`](../src/domain/review.ts). The diff UI in
[`src/components/FileDiff.vue`](../src/components/FileDiff.vue) presents those two choices and
the export/send button. The choice is persisted per checkout under `review_target:{checkout_id}`;
missing or unrecognized values resolve to Markdown. `review_target_load` and
`review_target_save` are registered IPC commands in
[`src-tauri/src/commands/ui_state.rs`](../src-tauri/src/commands/ui_state.rs), backed by
[`src-tauri/src/persistence/mod.rs`](../src-tauri/src/persistence/mod.rs).

[`src/App.vue`](../src/App.vue) builds the Markdown using `buildReviewMarkdown` from
[`src/domain/review.ts`](../src/domain/review.ts), supplies the local date from
`localReviewTimestamp`, and dispatches according to the selected target. The diff's sendable-note
filter includes draft and sent notes, excludes resolved notes, and omits outdated notes unless the
user opts in. The Markdown format contains the review heading, branch range, file paths, note
anchors, and note text.

For Markdown, the app calls `review_export_markdown` through
[`src/lib/ipc.ts`](../src/lib/ipc.ts) and
[`src-tauri/src/commands/files.rs`](../src-tauri/src/commands/files.rs). The service in
[`src-tauri/src/services/files.rs`](../src-tauri/src/services/files.rs) writes under
`$HOME/.marvis/tmp/code-reviews/`, creating the directory when needed. The filename is
`YYYY-MM-DD-HHMM.md`; collisions receive `-2`, `-3`, and so on via exclusive file creation, so an
existing export is never overwritten. The backend validates the date and timestamp before using
them in a path, and rejects invalid dates, NUL bytes, and Markdown larger than 512 KiB.

The returned relative filename opens as a document with `origin: "review"`. That origin is carried
through [`src/domain/main-document.ts`](../src/domain/main-document.ts), persisted UI state, and
the file read/write IPC. The Rust file service accepts only relative `.md` review paths and
canonicalizes them against the review root; writes also compare the expected current contents and
replace atomically. Copy-path resolution uses the review root. The review origin is stored in
`PersistedDocument`; the current database schema version is 13.

## OpenCode rounds are a separate contract

The OpenCode branch in `src/App.vue` calls `dispatchRound` in
[`src/presentation/review-notes.ts`](../src/presentation/review-notes.ts); Markdown export does
not call it. The OpenCode round model is `ReviewRound` in
[`src-tauri/src/domain/review.rs`](../src-tauri/src/domain/review.rs), and lifecycle behavior is
implemented by [`src-tauri/src/services/review_round.rs`](../src-tauri/src/services/review_round.rs):

- A round targets an OpenCode session and carries its note IDs, status, and unique marker.
- `begin_round` records `dispatching` and marks notes `sent` before attempting delivery; queued
  rounds retain the accepted prompt for later flushing.
- The prompt includes `AGENT_PROMPT_PREFIX` and the round marker. Reconciliation checks the
  session transcript and confirms or requeues an interrupted send.
- An export creates no round, adds no marker, and does not change note status or the unfinished
  round count.

The available automated session integration is OpenCode only. `ReviewTarget` does not include
`codex`, `claude`, or `pi`; the follow-on documents above remain specifications, not implemented
capabilities.

## Verification references

Relevant source tests cover export collision/date validation and review-root containment in
`src-tauri/src/services/files.rs`, review-origin persistence in
`src-tauri/src/persistence/mod.rs`, and frontend target/export/document behavior in the corresponding
`src/` test files. This is source evidence, not a claim that those tests were run for this audit.

[Agent live-test guidance](agent-live-tests.md) documents the opt-in OpenCode process/provider
tests and the hermetic review-round checks. Export itself starts no process and requires no provider.
