/**
 * What a person actually sees when a terminal opens.
 *
 * Marvis sets a terminal's shell integration up by typing one line at it, and the script that line
 * sources finishes by clearing the screen. Neither is visible: the line is drawn by the tty and taken
 * back off, and so is everything the shell printed while starting up.
 *
 * None of that can be checked from the byte stream. The bytes are in the stream whether or not they
 * were honoured, so a stream-level assertion passes just as happily when a terminal opens with a gap
 * at the top and the setup line still on it. Only a rendered screen tells a line that was drawn and
 * taken back off from one that was drawn and left, which is why this file exists and why it uses a
 * real terminal rather than a string search.
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
 * Regenerating always produces a diff: the captures embed the prompt's clock. That is expected.
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

/** The shells the app opens a terminal with, and which is which. */
const SESSIONS = ["zsh-login", "zsh-bare", "bash-login"];

function capture(name: string): Uint8Array {
  return new Uint8Array(readFileSync(join(CAPTURES, `${name}.bin`)));
}

/**
 * Everything the terminal draws, one string per row.
 *
 * A real emulator and not a screen-scraping helper: the setup line is echoed by the tty line discipline
 * and then cleared with cursor movement and erase-display, so the only thing that can say whether any
 * of it survived is something that keeps a screen.
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

/** The last row with anything on it, or -1 for a screen that drew nothing. */
function lastContentRow(rows: string[]): number {
  return rows.map((row) => row.trim().length > 0).lastIndexOf(true);
}

/** The blank rows between the top of the screen and the last thing on it. */
function interiorBlankRows(rows: string[]): string[] {
  const last = lastContentRow(rows);
  const blank: string[] = [];
  for (let y = 0; y <= last; y++) {
    if (rows[y]!.trim().length === 0) blank.push(String(y));
  }
  return blank;
}

describe("what a terminal shows after it sets up shell integration", () => {
  it("opens pristine: contiguous from the top, with nothing of the setup left on it", async () => {
    for (const name of SESSIONS) {
      const rows = await renderedRows(capture(name));
      // No blank row anywhere between the top of the screen and the last thing on it. This is the
      // assertion the clear earns its place with, and it is the *interior* blanks that matter:
      // erasing the row the setup line was drawn on removed the text and left the row, so a terminal
      // whose prompt is taller than one row opened with the prompt's first rows, then blanks where the
      // setup had been, then the first command. A gap at the very top would have been the easier thing
      // to notice and the easier thing to assert, and it is not what actually went wrong.
      expect(interiorBlankRows(rows), `${name} opened with gaps in it`).toEqual([]);
      // And nothing of the setup itself, which is the half an erase already got right and which a
      // clear must not give back.
      expect(
        rows.filter((row) => row.includes("hook.zsh")),
        `${name} still shows the setup line`,
      ).toEqual([]);
    }
  });

  it("would open with gaps and the setup line if the script did not clear, which is what makes that mean something", async () => {
    const rows = await renderedRows(capture("control-no-clear"));
    // The same session, the same shell, the same line, with only the clear left out: the sourced line
    // still on it, and the blank row the clear removes still sitting above the first command.
    expect(rows.filter((row) => row.includes("hook.zsh")).length).toBe(1);
    expect(interiorBlankRows(rows)).not.toEqual([]);
  });

  it("still opens clean when the line was written before the shell settled", async () => {
    // The clear covers the whole screen, so it also covers the second render a line typed too early
    // gets — the tty echoes it in canonical mode and the line editor draws it again once it takes
    // over. The startup settle in `terminal/mod.rs` still waits for the shell to be ready, and this
    // says the clear does not depend on that having worked: one clear, and the terminal is clean.
    const rows = await renderedRows(capture("control-no-settle"));
    expect(interiorBlankRows(rows)).toEqual([]);
    expect(rows.filter((row) => row.includes("hook.zsh"))).toEqual([]);
  });

  it("reports a failed command after clearing itself", async () => {
    // The half that must not be traded away for a terminal that opens pristine: the markers are in the
    // capture because the capture waited for them, and this says so of the bytes themselves so the
    // assertion is about the stream rather than about a test having passed.
    for (const name of SESSIONS) {
      const stream = new TextDecoder().decode(capture(name));
      expect(stream, `${name} never reported its failing command`).toContain("\x1b]133;D;1\x07");
    }
  });

  it("reports the command that installed it as a clean exit, and says a command is starting", async () => {
    for (const name of SESSIONS) {
      const stream = new TextDecoder().decode(capture(name));
      expect(stream, `${name} never reported the install`).toContain("\x1b]133;D;0\x07");
      expect(stream, `${name} never announced a starting command`).toContain("\x1b]133;A\x07");
      // And the clear is in the stream, which is what the control capture above varies.
      expect(stream, `${name} never emitted the clear`).toContain("\x1b[3J\x1b[H\x1b[2J");
    }
  });

  it("shows the failing command and its prompt, so the capture is a real session and not an empty one", async () => {
    for (const name of SESSIONS) {
      const rows = (await renderedRows(capture(name))).filter((row) => row.trim().length > 0);
      expect(rows.join("\n"), `${name} rendered nothing`).toMatch(/false/);
      // A shell sitting at a prompt is the only thing a terminal opens with; if the prompt is missing
      // then the clear ran after it was drawn and took it with it.
      expect(rows.at(-1), `${name} did not open at a prompt`).toMatch(/[%>$#❯]/);
    }
  });
});
