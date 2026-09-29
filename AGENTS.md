# AGENTS.md

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
major stays `0` belongs to the person running it, not to the script.

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
