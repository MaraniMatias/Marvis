# Terminal pipeline spike (MVP 0.2)

An isolated Tauri 2 experiment, not the Marvis app scaffold. The path under test is:

```text
portable-pty reader (64 KiB chunks)
  → Rust TerminalBackend
  → Tauri Channel<tauri::ipc::Response> (raw bytes)
  → Channel<ArrayBuffer> in the WebView
  → xterm.js 6 renderer
```

The PTY backend is a separate crate from Tauri and xterm. Its output sink is injected, so the same PTY code can be exercised without a WebView. The frontend has single-shell tests plus a five-pane mode; terminal output is not sent as global UI events.

## Run the automated checks

From this directory:

```sh
pnpm install
pnpm test
pnpm build
cargo build --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path terminal-backend/Cargo.toml -- --nocapture --test-threads=1
cargo test --manifest-path terminal-backend/Cargo.toml yes_streams_for_thirty_seconds -- --ignored --nocapture --test-threads=1
```

`pnpm-workspace.yaml` explicitly approves esbuild's and vue-demi's install scripts for pnpm 12. `pnpm test` uses Node's test runner and xterm's headless parser; it does not exercise a WebKit WebView. The ignored Rust test is intentionally a full 30-second `yes` run.

To launch the macOS app:

```sh
pnpm tauri dev
```

The frontend is Vue 3 + TypeScript + Vite, styled with Tailwind and local shadcn-vue-style controls using Reka UI tooltip primitives. The shell starts in the spike directory. Use **Start shell** for the single-session tests, or **Start five PTYs** for five independent shells, channels, and xterm views. The latency probe uses the shell's `printf`; the exact-payload probe uses Node.js to write the requested byte count between unique markers. The `yes` button runs for 30 seconds from its first output line, then sends Ctrl-C.

## Manual WebView checklist

Run `pnpm tauri dev`, keep the native app visible, and record the values shown by the UI; do not treat Rust-only results below as WebView results.

1. Start one shell. Confirm the shell prompt, current directory, typing, Unicode, color, paste, and command completion.
2. Click **Measure PTY round-trip** at an idle prompt; record the displayed write → PTY → Channel latency.
3. Click **Measure exact payload** at 16 MiB. Record byte count, missing/corrupt bytes, raw MiB/s, xterm parser completion, frame gap, backend dropped bytes, and final channel byte delta.
4. Click **Run yes · 30s** and leave the app foregrounded. Record duration, throughput, invalid bytes, maximum UI frame gap, backend dropped bytes, and channel delta after it drains. Try typing/resizing while it runs; note whether controls respond.
5. Create disposable files and stream/search them without touching project files:

   ```sh
   tmp="$(mktemp -d)"
   node -e 'require("node:fs").writeFileSync(process.argv[1], "large cat test payload 0123456789\\n".repeat(100000))' "$tmp/large.txt"
   printf 'alpha\nneedle\nomega\n' > "$tmp/search.txt"
   cat "$tmp/large.txt"
   rg -n needle "$tmp/search.txt"
   rm -rf "$tmp"
   ```

6. Run `cargo build --manifest-path src-tauri/Cargo.toml` and `pnpm install --frozen-lockfile` in the shell. Confirm output stays in the terminal and the window remains usable.
7. Run `nvim --clean` and quit with `:q`; then run `htop` and quit with `q`. Resize the app rapidly while each is open; confirm the TUI follows the terminal size.
8. Click **Start five PTYs**. Verify all five panes receive their distinct `MARVIS_FIVE_n` output; type a command in each and resize the window repeatedly. Record channel byte delta and whether any pane stops updating. Close all five when done.
9. Use Activity Monitor to record CPU and memory for the app and its `WebKit Web Content` process(es), both idle and during `yes`. The Rust `ps` sample below is for the `yes` child only and is not app/WebView resource usage.

The UI probe values and all UI checklist outcomes are deliberately left unclaimed until someone can operate and observe the native window.

## Headless macOS measurements captured here

Environment: macOS 26.6.2, arm64; Rust 1.98.1 / Cargo 1.98.1; Tauri 2.11.6; Node 24.18.0 / pnpm 12.5.1; xterm.js 6.0.0; `yes`, `rg`, `nvim`, and `htop` available on `PATH`. These are one local run, not performance guarantees.

| Check | Observed | What it establishes |
| --- | --- | --- |
| Finite large `cat` | 16,777,216 bytes byte-for-byte equal; 144.8 MiB/s; backend dropped 0 B | PTY reader/callback path preserves the known payload in this headless run. |
| `yes`, 30.007 s | 3,370,848,865 bytes; 107.13 MiB/s; backend read/sent equal; callback drops 0 B | Sustained PTY reader throughput and no callback failures. `yes` has no sequence/checksum, so this cannot detect a missing whole line. |
| `yes` child resources | `ps` samples: 74.5–75.8% CPU, 1,168 KiB RSS | Child process only; not the Rust app, WebView, or total system CPU. |
| Input echo, headless PTY | 40 writes; median 21 μs, p95 53 μs | PTY write-to-line-discipline-echo latency only; excludes Tauri IPC and WebView. |
| Resize, headless PTY | 200 rapid `TIOCSWINSZ` updates; median 4 μs, p95 6 μs; final dimensions read back correctly | Native PTY resize call only; excludes channel/UI/render time and interactive app redraw. |
| Five concurrent PTYs | Five `cat` PTYs returned distinct echoes; all remained independently writable; 0.09 ms until echoes were observed | Rust/backend concurrency, not five WebView renderers. |
| `rg`, `nvim --clean`, `htop` | `rg` matched a path with spaces; `nvim` and `htop` both emitted PTY output and accepted resize to 91×31 | Headless process/PTY compatibility smoke tests, not visual usability checks. |
| `cargo build`, `pnpm install` over PTY | Cargo 598 PTY bytes / 3.63 s; pnpm 194 PTY bytes / 0.05 s; expected completion output observed | Both commands were spawned as argument arrays under portable-pty and their output drained through the backend callback. |
| xterm headless parser | Split UTF-8 and ANSI output rendered as expected | xterm parser behavior only; not WebKit layout, paint, or frame rate. |
| Build/install | `cargo build`, `pnpm install`, `pnpm build`, `pnpm test` passed | Project compiles and dependencies install; does not establish interactive runtime correctness. |

### UI measurement gap

`pnpm tauri dev` did launch the native `terminal-spike` process. This session could not inspect or operate its window: macOS accessibility automation returned `osascript is not allowed assistive access (-1728)`, and `screencapture` could not create an image from the display. Consequently no Tauri Channel delivery count, WebView byte-drop result, nvim/htop visual check, resize-to-redraw latency, frame gap, UI responsiveness, or app/WebKit CPU/RAM result is reported. Complete the manual checklist before deciding the terminal spike passes; in particular, a Rust-only 30-second `yes` pass is not a WebView pass.

For this frontend migration, the available desktop browser integration reported that no browser was connected to the session. It targets browser tabs, not Tauri's native WebKit window or its IPC bridge, so it cannot substitute for native WebView end-to-end testing.
