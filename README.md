# Marvis

A focused workspace for repositories and checkouts: browse files, review diffs, leave notes, and keep a terminal per checkout.

## Install

Download the `.dmg` from the [latest release](https://github.com/MaraniMatias/Marvis/releases/latest) and drag Marvis into Applications.

The app is not code-signed, so Gatekeeper warns on first launch. Open it with right-click → Open, or:

```sh
xattr -dr com.apple.quarantine /Applications/Marvis.app
```

On Debian and Ubuntu, download the `.deb` from the same release and install it:

```sh
sudo apt install ./Marvis_0.1.0_amd64.deb
```

The `.AppImage` from the release covers the rest of the desktop Linux. It needs no install:

```sh
chmod +x Marvis_0.1.0_amd64.AppImage
./Marvis_0.1.0_amd64.AppImage
```

## Build from source

Requires Node 22, pnpm and Rust.

```sh
pnpm install
pnpm dev:app   # run the app
pnpm build:app # bundle for the host platform
```

`pnpm build:app` writes into `src-tauri/target/release/bundle`, and what lands there depends on the host: a `dmg` on macOS, a `deb` and an `AppImage` on Linux. The bundler takes the list in `bundle.targets` and keeps only the targets the current platform can produce, so one config covers both without a per-OS override.

On Linux the bundler needs the GTK and WebKit development packages, which are not in a default install:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Building the AppImage additionally downloads `linuxdeploy` into the Tauri cache directory, so that one target needs network access. Pass `--bundles deb` to skip it.

## License

[MIT](LICENSE)
