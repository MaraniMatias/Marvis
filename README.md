# Marvis

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux-lightgrey)
![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB)

Marvis is a desktop app for working with repositories and their checkouts. You can browse files, review diffs, leave notes, and keep a separate terminal for each checkout.

Files open with syntax highlighting, and Markdown renders with task lists. Diffs have their own review view. Each terminal runs on xterm.js with WebGL rendering, so switching between checkouts does not mean juggling shell tabs.

<!-- Add a screenshot or short GIF here, for example docs/screenshot.png -->

## Install

Download the build for your system from the [latest release](https://github.com/MaraniMatias/Marvis/releases/latest).

### macOS

Download the `.dmg` and drag Marvis into Applications. The app is not code-signed, so Gatekeeper warns on first launch. Open it with right-click, then Open, or remove the quarantine flag:

```sh
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

### Debian and Ubuntu

```sh
sudo apt install ./Marvis_<version>_amd64.deb
```

### Other Linux distributions

The `.AppImage` runs without installation:

```sh
chmod +x Marvis_<version>_amd64.AppImage
./Marvis_<version>_amd64.AppImage
```

## Build from source

You need Node 22, pnpm, and Rust.

```sh
pnpm install
pnpm dev:app     # run the app in development mode
pnpm build:app   # bundle for the host platform
```

`pnpm build:app` writes into `src-tauri/target/release/bundle`. The result depends on the host: a `.dmg` on macOS, a `.deb` and an `.AppImage` on Linux. The bundler reads the list in `bundle.targets` and keeps only the targets the current platform can produce, so one config works on both.

On Linux, the bundler needs the GTK and WebKit development packages, which a default install does not include:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

The AppImage target also downloads `linuxdeploy` into the Tauri cache, so it needs network access. To skip it, build only the deb:

```sh
pnpm build:app --bundles deb
```

## Development

| Command | Purpose |
| --- | --- |
| `pnpm dev:app` | Run the desktop app with the `dev-bridge` feature enabled |
| `pnpm dev` | Run the Vite frontend alone on `127.0.0.1:1420` |
| `pnpm typecheck` | Type-check the Vue and TypeScript code |
| `pnpm lint` | ESLint on the frontend |
| `pnpm lint:rust` | Clippy on the backend, with warnings treated as errors |
| `pnpm fmt` | Format the frontend and the Rust code |
| `pnpm fmt:check` | Check formatting without changing files |
| `pnpm test` | Frontend tests (Vitest) and backend tests (`cargo test`) |
| `pnpm test:security` | Only the security tests: `SEC-` in the frontend, `sec_` in Rust |

The app is a Tauri 2 shell with a Rust backend and a Vue 3 frontend written in TypeScript. The frontend uses Vite, Tailwind CSS 4, and Reka UI. CodeMirror 6 and Shiki handle code, markdown-it (sanitized with DOMPurify) handles Markdown, and `@git-diff-view/vue` draws the diffs.

### Layout

| Path | Contents |
| --- | --- |
| `src/` | Vue frontend |
| `src-tauri/` | Rust backend and Tauri configuration |
| `docs/` | Project documentation |
| `scripts/` | Helper scripts |
| `git-spike/`, `layout-spike/`, `terminal-spike/` | Spike projects for the git, layout, and terminal features |
| `.github/workflows/` | CI and release workflows |

### Agent access

`pnpm dev:app` builds with the `dev-bridge` Cargo feature and loads `src-tauri/tauri.dev-bridge.conf.json`. The repository's `opencode.json` registers the [`@hypothesi/tauri-mcp-server`](https://www.npmjs.com/package/@hypothesi/tauri-mcp-server) MCP server, so an MCP-capable agent can work with the running development build.

## Contributing

Before opening a pull request, run:

```sh
pnpm fmt:check && pnpm lint && pnpm lint:rust && pnpm typecheck && pnpm test
```

## License

[MIT](LICENSE)
