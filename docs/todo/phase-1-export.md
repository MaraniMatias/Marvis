# Phase 1 — Markdown export as a review destination

Status: specified, not implemented. This is the file an implementer works from. Every claim about
the current code carries its `file:line`; anything I could not verify is written "unconfirmed"
rather than guessed.

The app today has exactly one destination for a review: a button in the diff that hands the chosen
notes to a per-checkout `opencode serve` process as one message
(`src/components/FileDiff.vue:484`, `src/App.vue:883`,
`src-tauri/src/commands/review.rs:202`). Phase 1 adds a second one — write the same Markdown to a
file and open it in Marvis's own file view — and makes the destination a per-checkout setting.

## 1. Goal and non-goals

**Goal.** A user with no OpenCode, or with OpenCode and a different agent they actually use, can
press the same button in the same diff and get their review as a Markdown file on disk that they
can read, edit, copy, or paste into whatever they like.

**Non-goals, explicitly:**

| Not in phase 1                    | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Codex, Claude Code or pi          | Each is its own phase (`docs/phase-2-codex.md`, `docs/phase-3-claude-code.md`, `docs/phase-4-pi.md`), decided one harness at a time.                                                                                                                                                                                                                                                                                                                        |
| A `ReviewRound`                   | Writing a file is not evidence that anyone was handed the review. A round means "sent to an agent", and the round machinery exists so an interrupted send is recognizable (`src-tauri/src/services/review_round.rs:1-6`). A file is either there or it is not.                                                                                                                                                                                              |
| Marking notes `sent`              | Same reason. `sent` is the backend's claim that an agent received them (`src-tauri/src/services/review_round.rs:52-55` marks notes in `begin_round`, before the agent is called). A file on disk makes no such claim.                                                                                                                                                                                                                                       |
| The "N rounds not finished" count | It counts rounds whose status is not `acked` (`src/App.vue:937-939`). With no round, there is nothing to count. The count stays hidden when the target is `markdown`.                                                                                                                                                                                                                                                                                       |
| `foregroundApp` detection         | `src-tauri/src/terminal/process.rs` can name the program in front of a terminal — `proc_pidpath` on macOS (`:21-34`), `/proc/<pid>/comm` on Linux (`:55`) — and it surfaces as `foregroundApp` (`src-tauri/src/domain/workspace.rs:89`, `src/domain/workspace.ts:14`). It says `codex`, not which conversation that codex has open. Naming a program is not addressing a session, and Marvis has no authorization to write into a session the user started. |
| Listing exported files somewhere  | The file does not appear in the checkout's `git status` and not in Marvis's file tree. It is reached from the diff, the way it was just written.                                                                                                                                                                                                                                                                                                            |
| Cleaning up old exports           | Nothing in phase 1 deletes `~/.marvis/tmp/code-reviews/`.                                                                                                                                                                                                                                                                                                                                                                                                   |
| Any SQLite change                 | See §7.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

## 2. The user flow

1. The user is in a diff with notes. The header shows `N notes · M drafts`, a destination control,
   the "Send to opencode" button, and the "include outdated" checkbox
   (`src/components/FileDiff.vue:482-602`).
2. The user opens the destination control and picks **Markdown**. The choice is written as
   `preferences.review_target:{checkout_id}` and it sticks per checkout.
3. The button's label changes. "Send to opencode" is a lie when nothing is sent to OpenCode, so
   with `markdown` selected it reads **Export as Markdown**.
4. Pressing it writes the file. Writing interrupts nothing: no session is chosen, no agent is
   polled, no turn is interrupted, and no busy-agent dialog appears. There is no "send now vs
   queue" question to ask about a target that cannot be busy.
5. The diff calls `showView(checkoutId, { kind: "document", path, mode: "view" })`
   (`src/App.vue:389-395`). The file view opens on the new file, rendered as Markdown
   (`mode: "view"` is what `openFileDocument` picks for a `.md` path).
6. The user edits it in the editor or copies it and pastes it into their agent. Either way the
   export has done its job and Marvis forgets about it.

The file does not reappear on restart as a _document_ unless the user leaves the view open when
they quit, in which case the persisted `checkout_ui_states` row restores it (see §4).

## 3. The file contract

| Property       | Value                                            | Note                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Directory      | `~/.marvis/tmp/code-reviews/`                    | Created on demand. A **second allowlisted root**, not a subdirectory of any checkout.                                                                                                                                                                                                                                                                                                      |
| Filename       | `2026-03-14-1532.md`                             | `{local date}-{HHMM}.md`, 24-hour, local time.                                                                                                                                                                                                                                                                                                                                             |
| Heading        | `# Code Review 2026-03-14`                       | The date, no minutes.                                                                                                                                                                                                                                                                                                                                                                      |
| Collision      | `2026-03-14-1532-2.md`, `-3`, …                  | **Never overwrite.** See below.                                                                                                                                                                                                                                                                                                                                                            |
| Size limit     | 1 MiB                                            | `MAX_FILE_BYTES` in `src-tauri/src/services/files.rs:21`. The review is the same text the OpenCode path bounds at 512 KiB with `MAX_ROUND_PROMPT_BYTES` (`src-tauri/src/domain/review.rs:7`), so the tighter of the two governs; use the same `too_large()` error the file view already maps to "This file is larger than the preview size limit" (`src/components/DocumentPane.vue:218`). |
| Notes included | The same `sendableNotes` the OpenCode path sends | Draft and `sent`, excluding `resolved`; excluding `outdated` unless the checkbox is ticked (`src/components/FileDiff.vue:178-183`, `isReviewableNote` at `src/domain/review.ts:190`).                                                                                                                                                                                                      |
| Marker line    | **None**                                         | No `marvis-review:` marker and no `AGENT_PROMPT_PREFIX` (`src-tauri/src/domain/review.rs:61-64`). There is no round to reconcile.                                                                                                                                                                                                                                                          |

**Heading and filename differ on purpose.** The filename carries minutes so two exports in the
same minute do not land on the same name; the title is a date, because a heading is what a human
reads. They are not meant to agree.

**Never overwriting is the important one.** An exported review is a file the user may have already
edited. Overwriting it would destroy work on a button press. So the writer probes
`{stem}.md`, then `{stem}-2.md`, `{stem}-3.md`, … and stops at the first name that does not exist.
It writes with an exclusive create (`OpenOptions::new().write(true).create_new(true)`) rather than
check-then-write, so two Marvis windows cannot race the same name; an `AlreadyExists` error is a
collision, not a failure, and the probe continues. Nothing in phase 1 ever truncates an existing
file at this path.

## 4. Opening it in the file view — the real work

This is the part that is underestimated. Today a document is identified by **`checkoutId` + a path
relative to that checkout**, and that path is checked in three separate layers before any byte is
read:

1. `parse_relative_path` — `src-tauri/src/services/files.rs:638`. Rejects empty, NUL, `..`, `/`
   roots, Windows prefixes, and any component named `.git`.
2. `resolve_checkout_path` — `src-tauri/src/services/checkout.rs:26`. Re-derives the checkout from
   its id, canonicalizes `checkout_root.join(relative)`, and requires the result to
   `starts_with(checkout_root)` (`:55-75`), answering `PathOutsideCheckout` otherwise.
3. `safe_checkout_relative_path` — `src-tauri/src/persistence/mod.rs:1899`. Bounds the stored
   string: non-empty, ≤ 4096 bytes, no backslash, every component `Normal`.

The exported file is at `~/.marvis/tmp/code-reviews/2026-03-14-1532.md`, which is **not** inside
any checkout. Reaching the file view therefore means threading a new _origin_ through the whole
path. Relaxing one check is not enough; each of these needs it.

| Place                                                                                                              | What it holds today                                                                                                                                            | What changes                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MainView`, `MainDocument` — `src/domain/main-document.ts:7-31`                                                    | `{ kind: "document"; path; mode }` and `MainDocument { checkoutId, path, source, mode }`                                                                       | Add `origin: "checkout" \| "review"` to both, defaulting to `"checkout"`. `mainViewToState` / `mainViewFromState` (`:46`, `:63`) must carry it or a restored review document comes back as a checkout path that does not exist.                                                                                                                                                                                                        |
| `PersistedDocument` — `src-tauri/src/persistence/mod.rs:75-80`                                                     | `checkout_id, path, source, mode`                                                                                                                              | Add `origin`. This is the stored JSON in `checkout_ui_states` and it is the only stored-shape change in this phase (§7).                                                                                                                                                                                                                                                                                                               |
| `validate_checkout_ui_state` — `src-tauri/src/persistence/mod.rs:1908-1918`                                        | `safe_checkout_relative_path(&document.path)` and `matches!(source, "file" \| "change")`                                                                       | Validate `origin`, and apply `safe_checkout_relative_path` **only** when `origin == "checkout"`. For `"review"`, validate the review-root shape instead: relative, `.md` suffix, no `..`, no backslash, no NUL.                                                                                                                                                                                                                        |
| `normalizeCheckoutUiState` — `src/domain/ui-state.ts:93-108`, `safePath` — `:62-65`                                | Every stored path goes through `safePath`, which rejects `""`, `>4096`, a leading `/`, and any `.`/`..`/empty segment                                          | Same split: `safePath` for a checkout document, a review-specific predicate for `"review"`. A review path like `2026-03-14-1532.md` passes `safePath` anyway, so the risk is not a false rejection here — it is a false **accept** if the path were ever something else, which is why the branch has to be explicit rather than reusing `safePath` and hoping.                                                                         |
| `file_read` — `src-tauri/src/commands/files.rs:40-51`; `files::read` — `src-tauri/src/services/files.rs:288-328`   | `checkoutId, path: PathBuf`; service runs `parse_relative_path` then `resolve_checkout_path`                                                                   | Add `origin`. On `"review"`: **skip `parse_relative_path`**, resolve against the Marvis review root instead of the checkout, and apply _the same_ canonicalize + `starts_with` containment against that root. Reuse the `PathOutsideCheckout` code so the existing user-facing message in `DocumentPane.vue:222-223` still applies.                                                                                                    |
| `file_write` — `src-tauri/src/commands/files.rs:54-67`; `files::write` — `src-tauri/src/services/files.rs:330-390` | same, plus the read-only refusal and the `expected_content` compare                                                                                            | Same origin branch, and **additionally accept only `.md`**. An origin that can write must be bounded: without the extension check, `origin: "review"` plus a symlink or a crafted name is a write primitive into a second tree. The `.md` limit is what keeps this origin from becoming a general file-write surface.                                                                                                                  |
| `readCheckoutFile` / `writeCheckoutFile` — `src/lib/ipc.ts:26-37`                                                  | `invoke("file_read", { checkoutId, path })`                                                                                                                    | Thread `origin` through both wrappers.                                                                                                                                                                                                                                                                                                                                                                                                 |
| `DocumentPane.vue`                                                                                                 | `props.path: string \| null`; `readCheckoutFile(checkoutId, path)` at `:430`; `writeCheckoutFile(checkoutId, path, …)` at `:355`; `copyFilePath` at `:235-245` | Take the origin, pass it on every read and write, and resolve the Marvis root instead of `checkout.canonicalPath` where a path is needed.                                                                                                                                                                                                                                                                                              |
| `absoluteFilePath` — `src/domain/files.ts:41-43`                                                                   | `canonicalPath + "/" + path`                                                                                                                                   | Without a review branch, "copy path" on an exported file yields `/Users/x/repo/2026-03-14-1532.md`, a path that does not exist. That is a small bug with a large cost: the one action a user takes with an export is copy it.                                                                                                                                                                                                          |
| `DocumentPane`'s draft / identity / language keys — `:45-46`, `:72`, `:90`                                         | `${checkoutId}\0${path}`                                                                                                                                       | **This is a real collision, not a cosmetic one.** With two roots, `checkout A / notes.md` and `~/.marvis/tmp/code-reviews/notes.md` produce the same key, and the user sees one file's draft while editing the other. Every key that embeds `path` must embed `origin` too. The repo already has a precedent for embedding a second component in these keys (`\0`, `DocumentPane.vue:72`), so this is a widening, not a new mechanism. |
| Markdown preview — `src/presentation/markdown-preview.ts:13-83`                                                    | `useMarkdownPreview(getCheckoutId)` resolves images through `readCheckoutMarkdownImage` (`:66`), which is checkout-scoped                                      | A review export has no images. It should not attempt them: an image reference in a review resolves to a checkout path that does not hold it, and the current code turns that into `markdownImageWarning = true` (`:76-77`) — a warning on a file Marvis itself wrote, which reads as a bug in the export. Skip image resolution entirely when `origin === "review"`.                                                                   |
| Git status "deleted" check — `DocumentPane.vue:62-67`                                                              | `gitSnapshot.status.files.some(f => f.path === props.path && f.status === "D")`                                                                                | A review path will never be in the checkout's status, so the check is harmless. Say so in a comment rather than leaving a reader to wonder.                                                                                                                                                                                                                                                                                            |

**No existing security guarantee is relaxed.** The app reads exactly two trees — the checkout and
`~/.marvis/tmp/code-reviews/` — and each gets its own containment check. `sec_05_2`
(`src-tauri/src/main.rs:290-369`) already asserts that every command goes through a service and
that no `commands/` module reaches for `fs::` itself; the new origin branch belongs in
`services/files.rs` for that reason, not in the command layer. `sec_01_02`
(`src-tauri/src/services/files.rs:827-853`) is the existing containment test and should be
extended, not bypassed.

**Where `~/.marvis` comes from is a decision, not a lookup.** The app's own data directory today
is Tauri's `app_data_dir()` (`src-tauri/src/main.rs:62-64`), which on macOS is
`~/Library/Application Support/<bundle id>`, not `~/.marvis`. Nothing in the repo resolves `~`:
`src-tauri/src/services/executable.rs:21-27` reads `$HOME` directly for its install-dir guesses.
Phase 1 needs one of: a new `paths` service that reads `$HOME` and joins, or the Tauri path
resolver. The literal `~/.marvis/tmp/code-reviews/` is a product decision; whichever resolver is
chosen, the review root must be computed once, canonicalized, and reused by both the writer and
the reader — two independent `~` resolutions that disagree are exactly the bug this phase is most
likely to ship.

## 5. The selector

`ReviewSender` (`src/presentation/review-notes.ts:82-92`) currently exposes `sessions`, `targetId`,
`selectTarget`, `unfinishedRounds` and `send(ids, queue)`. It gains:

- `target: ReviewTarget` — the checkout's destination, `"markdown"` by default.
- `selectTarget` becomes `selectTarget(target: ReviewTarget | string)`, or a new `selectReviewTarget`
  alongside it. Pick one and do not keep both: nothing here is deprecated, per `AGENTS.md`.
- `send(ids, queue)` keeps its signature. `queue` is meaningless for `markdown` and the caller
  never passes it in that case.

`ReviewTarget` is `"markdown" | "opencode"`. **No preference row means `markdown`.** That is a
deliberate behavior change: today the button always goes to OpenCode, so every existing user has
to pick OpenCode once and it sticks. `preferences` is a plain key/value table
(`src-tauri/src/persistence/mod.rs:2005-2008`) read through `get_preference` (`:2245`) and written
through `set_preference` (`:2256`); `review_target:{checkout_id}` needs one accessor pair, and
`set_preference` already handles the delete-on-`None` case. An unknown stored value must also read
as `markdown` — a value this build does not produce is not a value to guess at.

`src/App.vue` dispatches:

- `sendReviewToAgent` (`:883-916`) — unchanged for `opencode`: it resolves the session and calls
  `review.dispatchRound`, which records the round and sends. Untouched.
- A new branch for `markdown`: build the Markdown with an explicit local date, call the new
  export command, then `showView` the result. It does not call `dispatchRound` at all, so no
  round is created, no note changes status, and `unfinishedRounds` is unaffected.

`FileDiff.vue` is where the control lives. With `markdown` selected: the session `<select>`
(`:497-548`) and the busy dialog (`:554-585`) are not rendered — writing a file interrupts nothing
and there is no session to choose — and `unfinishedRounds` (`:586-592`) stays hidden, because
there are no rounds. `canSend` (`:190`) keeps meaning "there is at least one note to include", and
the button keeps being disabled when there is not.

**The Markdown is built once and sent once.** `buildReviewMarkdown`
(`src/domain/review.ts:131-156`) is the only producer; both destinations call it with the same
notes and the same `branch` / `defaultBranch` context, so an export and a round differ only in
what happens to the bytes afterwards.

## 6. The local-date bug

`buildReviewMarkdown` defaults its heading to
`new Date().toISOString().slice(0, 10)` (`src/domain/review.ts:135`). `toISOString` is **UTC**. At
21:00 in Argentina it is already the next day in UTC, so the heading says tomorrow.

That was survivable while the date was decoration inside a message. It is not survivable now that
the date **names a file**: the export lands in the wrong day's folder, next to a review the user
wrote last night, and the mistake is invisible unless you know the timezone.

Phase 1 passes an explicit `date` from the frontend, which already has the user's timezone:

- `date`: `{y}-{m}-{d}` from local components, zero-padded.
- `timestamp`: the same `y-m-d` plus local `HH` and `MM`.

Both go to the backend, and **the backend validates both against a strict shape before using either
in a path** — `^\d{4}-\d{2}-\d{2}$` and `^\d{4}-\d{2}-\d{2}-\d{4}$`. A value that fails is
refused, not sanitized: a validator that rewrites is a validator that has already decided the
caller was wrong.

Rationale for taking these as strings instead of computing them in Rust: **`src-tauri/Cargo.toml`
has no `chrono` and no `time`.** The crate has no date/time dependency at all, and JavaScript
already holds the answer in the right timezone, so the dependency is not worth adding. Strict
validation is precisely what makes accepting those strings safe — the backend never has to believe
the frontend about anything but two fixed shapes, and the filename it builds from them is fully
determined.

## 7. Persistence

**No SQLite change. `SCHEMA_VERSION` stays 11** (`src-tauri/src/persistence/mod.rs:17`). The
`preferences` table already exists and is already key/value; `review_target:{checkout_id}` is a
new _row_, not a new _table_, so `create_schema` (`:1949`) is not touched and no user's database is
refused. The `AGENTS.md` rule — one schema, no migrations, bump the version and let every
existing database start empty — does not apply, because nothing about the schema moves.

The one stored-shape change is the JSON inside `checkout_ui_states.state_json`: a new field on
`PersistedDocument`. `AGENTS.md` covers this case directly: _a breaking change to a stored shape
is made in place, and the version moves a minor_, with no deprecation window and no compatibility
layer, because nothing has to read what an older build wrote. So: no `#[serde(default)]` on
`origin` pretending to read an older build's JSON, no second field meaning the same thing. The
old shape is simply unreadable, and `validate_checkout_ui_state` (`:1908`) sending an unreadable
state to `CheckoutUiState::default()` (`:747-750`) is the existing, correct behaviour.

One consequence worth stating: `CheckoutUiState` itself already carries
`#[serde(default, …)]` (`:83`) for `diffAllFiles`, a field added after version 1 shipped, with a
comment saying so (`:88-90`). That is a **migration by another name** under the current reading of
`AGENTS.md` — a field that exists only to read what an older build wrote. Do not copy that pattern
for `origin`. If it turns out `diffAllFiles` is load-bearing for real users, that is a separate
conversation; this phase does not extend it.

**Version.** `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` are all at
`0.5.2` today, and `scripts/release.mjs` is the only thing that writes a version. The bump for
this phase moves the **minor**: `0.6.0`.

## 8. Tests

Named cases, following the repo's conventions — Rust security tests are `sec_NN_…`, frontend tests
read as full sentences.

**Rust — `src-tauri/src/services/files.rs`:**

- `sec_11_a_review_origin_reads_only_the_review_root` — `origin: "review"` on a path that resolves
  inside the checkout is refused; a path that escapes the review root with `..` or a symlink is
  refused with `PathOutsideCheckout`.
- `sec_12_a_review_origin_refuses_to_write_anything_but_markdown` — a `.txt` path and a
  no-extension path under the review root are both refused by `write`.
- `a_review_origin_never_reaches_outside_a_registered_checkout` — the checkout branch is unchanged:
  a review origin cannot be used to make `resolve_checkout_path` skip its containment check.
- `two_exports_in_the_same_minute_do_not_overwrite_each_other` — write twice with the same date and
  timestamp; the second lands on `-2`, and the first file's bytes are byte-identical afterwards.
- `a_review_export_with_a_malformed_date_or_timestamp_is_refused` — `2026-3-14`, `2026-03-14-999`,
  `../etc/passwd`, and a string over 4096 bytes are all rejected before any path is built.

**Rust — `src-tauri/src/persistence/mod.rs`:**

- `a_review_document_survives_a_restart_with_its_origin` — a `PersistedDocument` with
  `origin: "review"` round-trips through `save_checkout_ui_state` / `load_checkout_ui_state`.
- `a_document_saved_without_an_origin_is_dropped_not_guessed` — a `state_json` lacking `origin`
  does not come back as a checkout document.

**Rust — `src-tauri/src/main.rs`:**

- `sec_05_2_the_only_write_the_webview_can_reach_is_contained_by_the_checkout` — **must be updated
  or it will pass for the wrong reason.** It greps `services/files.rs` for the guards it knows
  about (`:358-363`). Its comment says the write is contained "by the checkout", which is no
  longer the whole claim. The assertion to add is that the review branch carries its _own_
  containment check, so the test still fails if a future edit drops it.

**Frontend — `src/domain/review.ts` / `review.test.ts`:**

- `uses the date it is given instead of the UTC one` — `buildReviewMarkdown(notes, { date:
"2026-03-14" })` heads the file `# Code Review 2026-03-14`; and with a fake timer set to
  21:00 local in a UTC+3 zone it does **not** produce tomorrow's date.
- `local_review_timestamp_is_zero_padded_across_the_hour_and_month_boundaries` — `09:05` and
  `00:00` on the 1st of the month.

**Frontend — `src/domain/main-document.test.ts` / `ui-state.test.ts`:**

- `round-trips a review document through the persisted state` — and the inverse: a `review` origin
  comes back as `review`, not as `checkout`.
- `keeps two same-named files in different roots as two different documents` — this is the
  draft-collision test in its purest form and it is the one that would have caught §4's identity-key
  row.

**Frontend — `src/components/FileDiff.test.ts`:**

- `exports the notes as markdown without a session, a busy dialog, or a round count` — asserts
  `send-target` and `send-busy` are absent and `unfinished-rounds` is absent when the target is
  `markdown`, and that the button is labelled for what it does.
- `the markdown target does not change which notes are included` — the same selection rules as the
  OpenCode path, including the outdated checkbox.

**Frontend — `src/components/DocumentPane.test.ts`:**

- `an exported file and a checkout file with the same name keep separate drafts` — the identity-key
  regression.
- `copying the path of an exported file yields the path that exists` — guards the
  `absoluteFilePath` branch.

## 9. Rollout

`README.md` says today, at line 16: _"With any other agent, you export the notes as Markdown and
paste them into its terminal."_ and at line 52 describes the OpenCode integration as the only
built-in one. Both are wrong the moment this ships — the export is now a destination _inside_ the
app, not advice the user has to follow by hand. Line 42's _"You can also export a round as
Markdown and paste it anywhere"_ describes an export that does not exist in the code today
(unconfirmed what it referred to; there is no export path in `src/`), and should be reconciled with
whatever this phase actually does.

`CHANGELOG.md` gets an `Unreleased` section. The honest headline is the **behavior change**: with
no preference row, the destination is Markdown, so an existing user pressing the button writes a
file instead of reaching OpenCode. That is a deliberate break, made in place, and it is the first
thing a returning user will hit.

`docs/agent-live-tests.md` is the model for how the six ignored OpenCode live tests document
themselves — what the env vars are, that the directories must be temporary and exclusive, and that
a missing required variable fails the test by name rather than passing silently. The markdown
export path has no live test in this phase; it is a file write against a real home directory and
there is nothing to negotiate. If one is added later, it follows that document's shape.

## Unverified / risks

- **`~/.marvis` is not a path anything in the repo already resolves.** `app_data_dir()` is
  `src-tauri/src/main.rs:62` and it is not that. Whether the review root lives at
  `~/.marvis/tmp/code-reviews/` or under the app data directory is a product decision this document
  takes as given; the _resolver_ is unspecified and unconfirmed.
- **The `tmp` in the path is a claim the code does not make good on.** Nothing prunes it, so the
  directory grows without bound and "tmp" invites a user or a tool to clean it. A review the user
  edited is not disposable. Confirm the name before it ships; `~/.marvis/reviews/` is the
  alternative, and it is a one-line change now and a data-migration argument later.
- **"The exported file does not appear in the file tree" is a design decision with a cost.** It
  keeps the user's repository clean, which is the point, but it also means a review the user
  exported a week ago is unreachable from Marvis. If that turns out to matter, the answer is a
  second allowlisted root with its own tree section, not writing the file into the checkout.
- **`PersistedDocument` gaining a field drops every saved `checkout_ui_states` row** on first run
  of the new build, because the old JSON cannot be read. That is the intended `AGENTS.md` behavior,
  and it is invisible — a user loses their last open document, not their notes — but it is a real
  change and belongs in the changelog.
- Whether `Markdown`'s heading should carry the branch or the checkout name alongside the date:
  **unconfirmed**, and out of scope. `buildReviewMarkdown` already prints
  `` `main..feature` — 2 files, 5 notes `` under the H1, which is more than a date needs.

## What a fake-process test proves

Nothing in this phase spawns a process, so the question does not arise here — and that is worth
saying plainly, because it is the one place phase 1 is simpler than phases 2 to 4. What the tests
in §8 _do_ prove is that our own path handling, containment and identity logic are correct against
a real filesystem in a temp directory. What they cannot prove is anything about a user's home
directory: whether `~/.marvis` is writable, whether it is inside a synced or backed-up folder, or
whether a second copy of Marvis is racing for the same filename. The `create_new` probe in §3 is
what turns that race from data loss into a suffix, and it is testable with two concurrent writers
in a temp dir — but only the race, not the environment.
