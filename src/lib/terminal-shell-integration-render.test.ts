/**
 * What a person actually sees when a terminal opens.
 *
 * The shell-integration setup is one line of input typed at a shell, and the line it types is erased
 * by the script that line sources. That erase cannot be seen from the byte stream: the bytes are
 * there whether or not they were honoured, so a stream-level assertion passes just as happily when a
 * line is left on screen. Only a rendered screen tells a line that was drawn from a line that was
 * drawn and then taken back off, which is why this file exists and why it uses a real terminal rather
 * than a string search.
 *
 * The captures are raw PTY output from real shells — a login zsh with oh-my-zsh and starship, a bare
 * `zsh -f`, a login bash — written by `captures_a_real_session_for_the_render_test` in
 * `src-tauri/src/services/terminal.rs`. Regenerate them with:
 *
 * ```sh
 * MARVIS_CAPTURE_FIXTURES="$PWD/src/lib/__fixtures__" \
 *   cargo test --manifest-path src-tauri/Cargo.toml captures_a_real_session -- --ignored
 * ```
 *
 * The control capture is the same session with the erase taken out of the script. It is the point of
 * this file: without it, "no rows show the line" could be an instrument that never saw the line.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import headless from "@xterm/headless";

const { Terminal } = headless;

const CAPTURES = join(dirname(fileURLToPath(import.meta.url)), "__fixtures__");

/** 80 columns and 24 rows: what the captures were made at, and what the app opens a terminal at. */
const COLS = 80;
const ROWS = 24;

function capture(name: string): Uint8Array {
  return new Uint8Array(readFileSync(join(CAPTURES, `${name}.bin`)));
}

/**
 * Everything the terminal draws, one string per row.
 *
 * A real emulator and not a screen-scraping helper: the injected line is echoed by the tty line
 * discipline and then erased with cursor movement, so the only thing that can say whether it is still
 * there is something that keeps a screen.
 */
async function renderedRows(bytes: Uint8Array): Promise<string[]> {
  const terminal = new Terminal({ cols: COLS, rows: ROWS, allowProposedApi: true, scrollback: 500 });
  terminal.write(bytes);
  // The trailing empty write resolves once everything queued before it has been parsed.
  await new Promise<void>((resolve) => terminal.write("", () => resolve()));
  const buffer = terminal.buffer.active;
  const rows: string[] = [];
  for (let y = 0; y < buffer.length; y++) {
    rows.push(buffer.getLine(y)?.translateToString(true) ?? "");
  }
  terminal.dispose();
  return rows;
}

describe("what a terminal shows after it sets up shell integration", () => {
  it("leaves no row with the injected line on screen, in any of the shells it supports", async () => {
    for (const name of ["zsh-login", "zsh-bare", "bash-login"]) {
      const rows = await renderedRows(capture(name));
      const offending = rows.filter((row) => row.includes("hook.zsh"));
      expect(offending, `${name} still shows the setup line`).toEqual([]);
    }
  });

  it("would show it if the script did not erase, which is what makes the assertion above mean something", async () => {
    const rows = await renderedRows(capture("control-no-erase"));
    // Exactly one row, and it is the sourced line: the same session, the same shell, the same line,
    // with only the erase left out.
    expect(rows.filter((row) => row.includes("hook.zsh")).length).toBe(1);
  });

  it("would show it if the line were written before the shell was ready, which is the other half", async () => {
    // The erase only reaches the row it is aimed at, so it cannot also reach the *other* render a line
    // typed too early gets: a shell that is still loading its startup files echoes in canonical mode,
    // and then draws the same line again with the line editor once it takes over. Two renders, one
    // erase, one of them left behind. Waiting for the shell is what stops there being two, which is
    // why `STARTUP_QUIET` in `terminal/mod.rs` is not a nicety.
    const rows = await renderedRows(capture("control-no-settle"));
    expect(rows.filter((row) => row.includes("hook.zsh")).length).toBeGreaterThan(0);
  });

  it("still reports a failed command after erasing itself", async () => {
    // The half that must not be traded away for an invisible setup line: the markers are in the
    // capture because the capture waited for them, and this says so of the bytes themselves so the
    // assertion is about the stream rather than about a test having passed.
    for (const name of ["zsh-login", "zsh-bare", "bash-login"]) {
      const stream = capture(name);
      const sawFailedCommand = new TextDecoder().decode(stream).includes("\x1b]133;D;1\x07");
      expect(sawFailedCommand, `${name} never reported its failing command`).toBe(true);
    }
  });

  it("reports the command that installed it as a clean exit, so the frontend is not left guessing", async () => {
    for (const name of ["zsh-login", "zsh-bare", "bash-login"]) {
      const stream = new TextDecoder().decode(capture(name));
      expect(stream, `${name} never reported the install`).toContain("\x1b]133;D;0\x07");
      // And the erase is in the stream, which is what the control capture above varies.
      expect(stream, `${name} never emitted the erase`).toContain("\x1b[A\x1b[2K\x1b[A\x1b[2K\x1b[B\x1b[B");
    }
  });

  it("shows the failing command and its prompt, so the capture is a real session and not an empty one", async () => {
    for (const name of ["zsh-login", "zsh-bare", "bash-login"]) {
      const rows = (await renderedRows(capture(name))).filter((row) => row.trim().length > 0);
      expect(rows.join("\n"), `${name} rendered nothing`).toMatch(/false/);
    }
  });
});
