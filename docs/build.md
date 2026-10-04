# Building

Install the locked JavaScript dependencies with `pnpm install --frozen-lockfile`, then build the native app with `pnpm build:app`. The production wrapper requires rustup, runs Rust 1.97.1 explicitly, validates both `rustc` and Cargo, and prints their versions before building. It passes `--locked` to Cargo and remaps this checkout's absolute source path to `.` and Cargo home to `.cargo` in compiler output, including paths containing spaces. Existing `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` are retained. This rewrites path prefixes only: it does not strip symbols or disable backtraces.

On Linux, the default bundle targets are `.deb` and AppImage. To build only the `.deb` (the CI Linux job's scope), pass the bundle selection through the wrapper:

```sh
pnpm run build:app -- --bundles deb
```

The release workflow's Ubuntu 22.04 AppImage builds install the additional `libfuse2` and `libgtk-3-0` packages; the base `.deb` prerequisites and exact runner scope are listed in [development setup](development.md#operating-system-dependencies).

By default, bundles are under `src-tauri/target/release/bundle/`; builds with an explicit Rust target triple use `src-tauri/target/<target>/release/bundle/`. A successful locked build is not a promise of bit-identical binaries across operating systems, linkers, SDKs, or system libraries. The CI Linux packaging job uses `ubuntu-22.04`; a macOS build does not cross-compile or verify Linux bundles.

The macOS bundle intentionally uses Tauri's `signingIdentity: "-"` for ad-hoc signing. It is not Developer ID-signed or notarized; changing that requires release signing credentials and a separate distribution decision. Release builds use the default Cargo feature set, so the optional development MCP bridge is excluded.

The production frontend checks also verify the Catppuccin and Fira Code notices/fonts. A scoped TypeScript-AST transform allowlists only the core, WebGL-addon, and Unicode-11-addon runtime modules. Within those modules it rewrites only the complete emitter throw containing the three ordered `disposed?`, `size?`, and `arr?` diagnostics followed by the exact unknown-listener `Error`. Each suppressed call still evaluates its arguments left-to-right and yields `undefined`, preserving effects such as `JSON.stringify` and its exceptions. Other logs, warnings, errors, lookalike prefixes, and unmatched throws remain untouched. The production frontend artifact check confirms those three diagnostic calls are absent and the exact exception is retained.
