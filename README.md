# Marvis

A focused workspace for repositories and checkouts: browse files, review diffs, leave notes, and keep a terminal per checkout.

## Install

Download the `.dmg` from the [latest release](https://github.com/MaraniMatias/Marvis/releases/latest) and drag Marvis into Applications.

The app is not code-signed, so Gatekeeper warns on first launch. Open it with right-click → Open, or:

```sh
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

## Build from source

Requires Node 22, pnpm and Rust.

```sh
pnpm install
pnpm dev:app   # run the app
pnpm build:app # bundle the .dmg into src-tauri/target/release/bundle/dmg
```

## License

[Apache-2.0](LICENSE)
