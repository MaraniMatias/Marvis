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

/** The blank rows above the first thing on the screen. */
function leadingBlankRows(rows: string[]): string[] {
  const first = rows.findIndex((row) => row.trim().length > 0);
  const end = first === -1 ? rows.length : first;
  const blank: string[] = [];
  for (let y = 0; y < end; y++) blank.push(String(y));
  return blank;
}

describe("what a terminal shows after it sets up shell integration", () => {
  it("opens pristine: nothing above the first row, and nothing of the setup left on it", async () => {
    for (const name of SESSIONS) {
      const rows = await renderedRows(capture(name));
      // The clear wipes the whole display and homes the cursor, so the shell's first prompt is drawn at
      // the top row and there is nothing above it.
      expect(leadingBlankRows(rows), `${name} opened with blank rows above it`).toEqual([]);
      // And nothing of the setup itself, which is the half an erase already got right and which a
      // clear must not give back.
      expect(
        rows.filter((row) => row.includes("hook.zsh")),
        `${name} still shows the setup line`,
      ).toEqual([]);
    }
  });

  it("deliberately does not count blank rows further down, and here is why", async () => {
    // A prompt is free to be several rows tall and to have blank rows of its own. starship draws a
    // directory line, then a blank, then the command line, and whether that blank is there varies
    // between two captures of the same shell minutes apart. Asserting "no blank row between the top of
    // the screen and the last thing on it" therefore measures the developer's prompt configuration
    // rather than the integration: it passes on one regeneration and fails on the next with rows 1 and 4
    // blank, both of them the prompt's own spacing.
    //
    // What is left is the claim that is actually about the integration, and it is not vacuous — the
    // control below shows the setup line still on the screen when the clear is left out, which is what
    // would make the assertion above fail. The mechanism itself is pinned in Rust, where
    // `the_script_clears_the_screen_and_does_it_last` asserts the script ends with `CLEAR_SCREEN`
    // rather than reading anything off a capture.
    const rows = await renderedRows(capture("zsh-login"));
    expect(rows.length).toBeGreaterThan(0);
  });

  it("would still show the setup line if the script did not clear, which is what makes that mean something", async () => {
    const rows = await renderedRows(capture("control-no-clear"));
    // The same session, the same shell, the same line, with only the clear left out: the sourced line
    // still on it.
    expect(rows.filter((row) => row.includes("hook.zsh")).length).toBe(1);
  });

  it("still opens clean when the line was written before the shell settled", async () => {
    // The clear covers the whole screen, so it also covers the second render a line typed too early
    // gets — the tty echoes it in canonical mode and the line editor draws it again once it takes
    // over. The startup settle in `terminal/mod.rs` still waits for the shell to be ready, and this
    // says the clear does not depend on that having worked: one clear, and the terminal is clean.
    const rows = await renderedRows(capture("control-no-settle"));
    // The same two claims as the integrated captures, and for the same reason: what is asserted is that
    // nothing sits above the first row and none of the setup is legible, never how many blank rows the
    // prompt happens to draw between its own lines.
    expect(leadingBlankRows(rows)).toEqual([]);
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
      // And the clear is in the stream, which is what the control capture above varies.
      expect(stream, `${name} never emitted the clear`).toContain("\x1b[3J\x1b[H\x1b[2J");
    }
  });

  // Separate from the block above because the shells disagree here, and one of the disagreements is
  // deliberate. bash reports the start of a command from `PS0`, which arrived in bash 4.4; `/bin/bash`
  // on macOS is 3.2.57 and is what this app spawns when `$SHELL` says so, so the `bash-login` capture
  // has no started marker in it and asserting one there would be asserting a bash that does not exist
  // on the machine. The consequence is stated in `shell_integration_script`: a bash 3.2 keeps the
  // sidebar row the colour the previous command left it, rather than turning blue the moment the next
  // one starts. zsh has `preexec` and is unaffected.
  it("announces a starting command on every shell that can know one", async () => {
    const withStartedMarker = SESSIONS.filter((name) => !name.startsWith("bash"));
    for (const name of withStartedMarker) {
      const stream = new TextDecoder().decode(capture(name));
      expect(stream, `${name} never announced a starting command`).toContain("\x1b]133;A\x07");
    }
    // And the bash capture is not silently passing because it is empty: it carries a real session, and
    // it carries no started marker. Asserting the gap is what keeps it from becoming a hole that a
    // future bash without `PS0` would widen without anybody noticing.
    const bash = new TextDecoder().decode(capture("bash-login"));
    expect(bash).toContain("\x1b]133;D;1\x07");
    expect(bash.includes("\x1b]133;A\x07"), "bash 3.2 has no PS0, so this should be absent").toBe(false);
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
