# AGENTS.md

Muster: a Tauri 2 desktop workspace for coding agents in Git worktrees. Vue 3 + TypeScript
frontend in `src/`, Rust backend in `src-tauri/`.

## Development and verification

- Use pnpm (pinned in `package.json`), Node 22 (CI), and the Rust toolchain in
  `rust-toolchain.toml`. Install with `pnpm install --frozen-lockfile`.
- `pnpm dev` starts only Vite on `127.0.0.1:1420`; `pnpm dev:app` starts the desktop app
  plus Vite with the optional MCP bridge. That bridge is behind the `dev-bridge` feature
  and must never appear in a release build.
- Focused frontend test: `pnpm exec vitest run src/App.test.ts -t "name"`.
- Focused Rust test: `cargo test --locked --manifest-path src-tauri/Cargo.toml <filter>`.
- Vitest runs in the Node environment by default; component tests opt into the DOM with a
  `// @vitest-environment happy-dom` line at the top of the file.
- `pnpm release:validate` runs version checks, formatting, lint, typecheck, Clippy and all
  tests. It does not include CI's security tests, coverage, production app build, or
  dependency audits.
- `pnpm test` covers frontend, release/audit scripts, Rust, and artifacts; the artifact
  step rebuilds the frontend. On Linux it also tests the vendored glib patch.
- `pnpm build` produces frontend assets only. Use `pnpm build:app` for production
  packaging: it enforces the pinned toolchain, `--locked`, and path remapping.
- On Debian/Ubuntu, `bash scripts/install-linux-deps.sh` installs the native prerequisites
  Rust tests and app builds need.

## Cross-layer changes

- IPC wrappers are `src/lib/ipc.ts`; the TS and Rust contracts are maintained separately in
  `src/domain/` and `src-tauri/src/domain/`.
- An IPC change spans the wrapper, `src-tauri/src/commands/`, handler registration in
  `src-tauri/src/main.rs`, and any capability permission it needs.
- Rust commands delegate to `src-tauri/src/services/`; blocking file work runs in
  `spawn_blocking`.
- The single-instance plugin is registered first on purpose: opening the database clears
  stored terminal sessions, so a second instance must stop before that happens.
- `src-tauri/vendor/glib-0.18.5` is a security backport, not vendored noise.
  `scripts/patched-glib.mjs` hash-checks its whole tree, so edits there break the audit gate.

## Versions are v0._._ — all of them

Every version this project ships is `v0.<minor>.<patch>`. The major is `0` and stays `0`
until someone says otherwise in a commit that also does the migration work described
below. A `v1.0.0` is not a version bump; it is the release where the schema rule below
has to already be in place, so it is its own deliberate act, not a consequence of a
`--bump`.

Consequences while the version is `v0.*.*`:

- A breaking change to a stored shape, an IPC contract, a preference key or a file
  format is made in place, and the version moves a minor. Nothing waits for a
  deprecation window, because nothing has to read what an older build wrote.
- There is no "deprecated but still supported" surface, no feature flag guarding a
  rewrite, and no compatibility layer held "one release" for anyone. The old path goes
  in the same commit as the new one.

`scripts/release.mjs` is the only place a version is written, and it does not enforce
the major by itself: bumping to `1.0.0` is a deliberate act, so the check that the
major stays `0` belongs to the person running it, not the script.

## Database schema: no migrations before 1.0

`src-tauri/src/persistence/mod.rs` carries one schema and no ladder. `create_schema`
writes the whole schema into a file that has none and refuses a file stamped with any
other `SCHEMA_VERSION`; a user upgrades by deleting the database and starting an empty
workspace. That is deliberate, and while the version is `v0.*.*` it stays deliberate.

While the version is `v0.*.*`:

- **Never write a migration.** No `ALTER TABLE`, no version ladder, no back-compat
  shim. Add the column to `create_schema`, bump `SCHEMA_VERSION`, ship it. Every
  existing database is refused by name and starts empty.
- **Never keep legacy code.** No deprecated variant kept readable, no
  `#[serde(default)]` on a field nothing writes yet, no branch for a shape the current
  build cannot produce. Delete the old path with the change.
- A field that exists only to read what an older build wrote is a migration by another
  name. Leave it out and fix the one place that wrote it.

The rule expires at 1.0. The first `v1.0.0` is where the ladder has to start: from that
version someone's work lives in that database and cannot be refused.

## Releases

- `pnpm release:preview v0.<minor>.<patch>` mutates nothing.
- Release `--dry-run` is not a local preview: it commits and pushes the version change and
  dispatches real GitHub bundle builds.
- Cuts require a clean `main` and an authenticated `gh`. Read `docs/release-safety.md`
  before touching this path; its command examples still say `bun run`, while the
  executable scripts and CI use pnpm.
- After a tag exists, recover through the original workflow run's failed jobs and its
  retained artifacts. Never move or delete a tag, and never rebuild a tagged release.
