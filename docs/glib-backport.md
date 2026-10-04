# GLib 0.18.5 security backport

Marvis vendors the crates.io `glib` 0.18.5 source because the GTK 0.18 dependency chain is not compatible with the fixed GLib 0.20 line. The unmodified crate archive had SHA-256 `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`, matching the checksum in the original `Cargo.lock`.

The only code change backports [gtk-rs/gtk-rs-core#1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343) (merge commit `05dff0ee696f9bcd8617cd48c4b812d046d440cb`) to `src/variant_iter.rs`: the out-pointer passed to `g_variant_get_child` is mutable (`let mut p` and `&mut p`). This fixes the UB behind `RUSTSEC-2024-0429` without changing the crate version or GTK-compatible API.

`scripts/patched-glib.mjs` pins the complete vendored tree—including its isolated test lockfile—to SHA-256 `cf39ca9b08530abb4d928d004438d2ad99a220339aefe2b5280663a8fb45aef0` and rejects symlinks and non-regular files. The Rust audit accepts the path package only at this exact directory and version. It labels the OSV finding fixed only when the complete identifier set is exactly `RUSTSEC-2024-0429` and `GHSA-WRW7-89JP-8Q8G`; all other findings still block at their normal severity. Linux `pnpm test:rust` runs the vendored iterator regression tests in release mode. The third-party notice generator reads the MIT license from this vendored source.

When updating the backport, verify the upstream fix, regenerate the complete-tree digest, and update the audit tests and this record in the same change. Do not broaden the accepted path, package version, or advisory identifiers.
