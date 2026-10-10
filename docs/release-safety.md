# Release safety

The release path is staged so no version is tagged until every check and final bundle has passed.

## Commands

- <code>bun run release:preview v0.20.0</code> shows the four manifest edits without changing files.
- <code>bun run release:validate</code> runs release:check, fmt:check, lint, typecheck, lint:rust, and the complete test suite on the current host. GitHub Actions is the canonical Linux and packaging gate.
- <code>bash scripts/validate-linux.sh</code> optionally builds x86_64 .deb/AppImage packages in Docker; each long container phase has a timeout.
- <code>bun run release v0.20.0 --dry-run</code> prepares the version, validates it locally, pushes the version commit to main, and runs checks plus every final GitHub bundle build. It never creates a tag or GitHub release.
- <code>bun run release v0.20.0</code> performs the same gates, then creates the remote tag and publishes the exact validated artifacts.

A dry run is not a local-only preview: it can push a version commit and start actual GitHub builds. Use release:preview for a no-mutation preview.

## Release sequence

1. From a clean main worktree, release.mjs prepares all four version records before local validation. release:check and the remaining gates therefore validate the requested version.
2. Validation may leave only those four expected manifest edits; any additional tracked or untracked change, staged change, HEAD movement, or branch change stops the release. If validation fails, the prepared version edits remain uncommitted for inspection; nothing is reset, committed, pushed, or tagged. Before retrying, restore any uncommitted manifest edits and remove validation-generated changes so the initial clean-worktree requirement is met.
3. After validation succeeds, release.sh commits only the version records and pushes the version commit to main without a tag. A non-fast-forward push stops safely; rebase or retry rather than resetting the branch.
4. The script dispatches the release workflow with the full commit SHA, version, mode, and a unique correlation ID. Every CI check and final build checks out that exact SHA even if main advances.
5. GitHub validates nonempty versioned artifacts for macOS ARM64 DMG and Ubuntu 22.04 x86_64/ARM64 .deb plus AppImage outputs. It rejects missing targets, duplicate names, version/architecture mismatches, and checksum or SHA manifest mismatches.
6. Only after all checks and builds succeed does the write-enabled publish job create the annotated tag and upload the already-validated payload. The workflow has no tag-push publication trigger.

## Retry and Docker behavior

A failed untagged build can be retried with the same version; it reuses the prepared commit and does not make a tag. If publication fails after tagging, rerun the original workflow's failed jobs so successful build artifacts are reused. A tagged same-run recovery skips checks and every build/finalization job, and publishes only from that run's retained final artifact; a missing or expired artifact fails before tag/release writes. Validation-only dispatch refuses any tagged version. The workflow validates the exact tag subject/run identity and never moves or deletes an existing tag. If artifacts cannot be recovered, stop and investigate rather than rebuilding an already-tagged version.

The optional local Linux check requires Docker. On Apple Silicon it runs Linux amd64 under emulation to match the Ubuntu x86_64 release runner. Source is streamed into an ephemeral container; host node_modules, Cargo targets, credentials, and Docker sockets are not mounted. The Docker image is only a local package-build smoke test; Linux tests run in GitHub Actions on the native runner. The release itself does not depend on a local Docker daemon; GitHub Actions validates and builds the exact pushed SHA before publishing.
