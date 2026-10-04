# Development

## Requirements

- Node.js 22 (the CI workflow uses Node 22).
- pnpm 12.5.1, pinned by `package.json`'s `packageManager` field. With Node's Corepack, run `corepack enable` and check `pnpm --version` from the repository; it should report `12.5.1`.
- Rustup with Rust 1.97.1. `rust-toolchain.toml` pins this toolchain and its `rustfmt` and `clippy` components. Install it with:

  ```sh
  rustup toolchain install 1.97.1 --component rustfmt --component clippy
  ```

The production build wrapper also explicitly invokes `rustup run 1.97.1`, validates `rustc` and Cargo, and requires Cargo's `--locked` mode. It does not use an unpinned `cargo` toolchain; see [build notes](build.md).

## Operating-system dependencies

Build on the platform you intend to package. The CI `.deb` job uses Ubuntu 22.04 and installs these Tauri GTK/WebKit prerequisites:

```sh
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

The release workflow also builds AppImages on Ubuntu 22.04 (amd64 and arm64) and installs these additional packages for AppImage bundling:

```sh
sudo apt-get install -y libfuse2 libgtk-3-0
```

These are the package lists used by the Ubuntu 22.04 CI/release runners, not universal prerequisites for every supported Linux distribution. On macOS, use a macOS host with Apple's Xcode Command Line Tools and SDK; the hosted CI runner provides these, and the workflow does not install additional Homebrew packages. Linux bundles are not cross-compiled or verified by the macOS job.

## Clone, install, and run

The configured origin is `git@github.com:MaraniMatias/Marvis.git`; HTTPS clone also works:

```sh
git clone https://github.com/MaraniMatias/Marvis.git
cd Marvis
corepack enable
pnpm --version # 12.5.1
rustup show active-toolchain # 1.97.1 from rust-toolchain.toml
pnpm install --frozen-lockfile
```

Run the desktop app in development mode:

```sh
pnpm dev:app
```

This runs Tauri's debug build with the `dev-bridge` feature and `tauri.dev-bridge.conf.json` overlay. That opt-in debug configuration enables the local MCP automation bridge; it is not part of a production release build.

Build the normal production app with:

```sh
pnpm build:app
```

This uses the pinned production wrapper and mandatory `--locked` Cargo build. On Linux, the default bundle targets are `.deb` and AppImage, so the AppImage runner extras above are needed. To build only the `.deb` with the CI base prerequisites, use `pnpm run build:app -- --bundles deb`. See [build and artifact notes](build.md) for bundle paths and build behavior. The dependency and runner commands above follow [CI](../.github/workflows/ci.yml) and the [release workflow](../.github/workflows/release.yml); they do not imply that CI is currently green.

## Workspace data and recovery

Marvis opens `marvis.sqlite3` from Tauri's `app_data_dir()` for app identifier `dev.marvis.workspace` (see `src-tauri/src/main.rs` and `src-tauri/tauri.conf.json`). The exact directory is platform-specific; do not assume a hard-coded macOS or Linux path. The database holds repository/checkout registrations, review notes and rounds, viewed-file and UI state, and window geometry/maximized state. User repository and worktree files are outside this database.

The current `v0.14.0` source uses schema 13. It refuses a nonzero database with another schema version, including schemas 11 and 12; it does not migrate it. Before 1.0, schema changes intentionally have no migration ladder. If you choose to delete an incompatible database, quit Marvis first and make a separate backup of `marvis.sqlite3`; Marvis does not create this backup for you. The next launch creates an empty workspace. This does not delete repository/worktree files, but it loses the Marvis registrations and metadata in that database. There is no automatic migration or in-app recovery. A schema-11 or schema-12 backup is still incompatible with this schema-13 build.

If a registered directory is temporarily unavailable, Marvis keeps the registration and marks it missing. When that same path returns, reconciliation clears the missing state on the existing checkout identity, leaving its notes and rounds attached. If a directory was moved or renamed, the current Sidebar offers only **Close missing**; reopening the new path creates a separate checkout identity and does not merge its history with the missing registration. Keep the old missing registration if that history matters.

Implementation note: the backend has an internal `locate_missing_checkout` IPC command/service, but the frontend has no wrapper or Sidebar action for it. It is not a supported end-user recovery workflow. When this internal re-key path is exercised, it refuses active terminal sessions, reserves the checkout against agent work, and transactionally transfers notes and round history after checking cross-checkout references; an active agent turn blocks it, while an idle bridge is stopped after a successful transfer. Dispatched and acked rounds retain status and their original session identity. Queued or dispatching rounds become `relocated`, retain their old session identity, and are no longer pending or requeued against the new checkout. Do not ask end users to invoke this internal command.

**Close missing** is not an archive: it explicitly removes the Marvis registration and associated database metadata. The UI asks for confirmation, but “No files will be deleted” means repository/worktree files; notes, rounds, and other metadata attached to the removed checkout are deleted. Closing a missing primary checkout also removes its registered checkout list. Back up `marvis.sqlite3` after quitting the app before confirming.

Archiving a live worktree is reversible: it leaves the directory and checkout registration in place, so existing notes remain. Marvis stops its per-checkout OpenCode bridge while the worktree is archived; restoring the worktree makes it available again but does not restart an agent. Worktree removal, archive, close, and moved-checkout re-key operations reserve affected checkout IDs, refuse active agents, and stop an idle bridge only after a successful change.

Terminal sessions and `checkout_terminal_layouts` are deliberately cleared at each startup because their PTY processes cannot survive the app process. Window geometry and saved app/per-checkout UI layout and view state remain across ordinary restarts. This is intentional, not a promise that live terminals can be restored.
