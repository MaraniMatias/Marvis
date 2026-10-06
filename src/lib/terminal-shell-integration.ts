/**
 * Whether the last command in a shell worked, as reported by the shell itself.
 *
 * The backend knows whether a *shell* exited and with what code, and that is the only failure it
 * can see: a command that fails the way people actually fail — `clang: error: no input files`,
 * `zsh: command not found: sd` — prints to the PTY and leaves the shell right where it was, so the
 * process never exits and there is no exit code to read. The failing command is a grandchild of the
 * process Marvis spawns, which `waitpid` cannot reach. The shell's own `$?` is the only source of
 * truth, and a shell only reports it if it is asked to.
 *
 * The asking is done once per session by `services/terminal.rs`, which writes OSC 133 markers to the
 * PTY: `D;<code>` after every command and `A` when the next one starts. This reads them back out of
 * the byte stream and says what they mean. Nothing here touches the backend: a command's exit code is
 * a fact about the shell, not about the session's process, which is the same reasoning that keeps
 * `terminalTitle` in the component rather than in the IPC contract.
 *
 * What `A` means is worth being precise about, because the protocol's name for it and the name it
 * carries here are not the same thing. `A` is *prompt start* in OSC 133; it is emitted from zsh's
 * `preexec_functions` and bash's DEBUG trap, which fire when a command is about to execute, so it is
 * used here as a proxy for "a command is starting". The marker for that is `C`, and neither it nor
 * `B` (prompt end, which no shell hook can report) is emitted, because producing them correctly needs
 * prompt state this integration does not track. That is a decision about who reads these bytes: the
 * only consumer is this handler, which keeps no prompt state, so `A` and `C` carry the same
 * information to it. `services/terminal.rs` documents the same from the producing side, and
 * `the_started_marker_fires_for_builtins_and_before_their_output` pins the position the marker is
 * emitted from, which is what makes the claim true.
 *
 * No Vue, so the parsing is testable on its own: what is worth pinning here is the payload grammar,
 * which the shell decides and this code only reads.
 */
import type { IDisposable, Terminal } from "@xterm/xterm";

/**
 * The OSC ident the shell integration markers arrive on.
 *
 * 133 is the ident every shell-integration script uses, and xterm.js ships no handler for it, so
 * registering one here contends with nothing the terminal already does with those bytes.
 */
const OSC_SHELL_INTEGRATION = 133;

/** What the shell last said about the command in front of it. */
export type ShellIntegrationEvent =
  /** A command finished; `exitCode` is its `$?`, so zero is success and anything else is a failure. */
  | { kind: "command-finished"; exitCode: number }
  /**
   * A command started, which means whatever failed before it is no longer what the user is watching.
   *
   * From the OSC 133 `A` marker, which the protocol defines as prompt start and this uses as a proxy
   * for command start; see the note at the top of this file.
   */
  | { kind: "command-started" };

/**
 * Reads the shell's exit codes out of the terminal's output.
 *
 * Returns the disposable that takes it back, because a terminal outlives most of what is registered
 * around it: a panel hiding, a session being replaced.
 */
export function registerShellIntegration(
  terminal: Terminal,
  onEvent: (event: ShellIntegrationEvent) => void,
): IDisposable {
  return terminal.parser.registerOscHandler(OSC_SHELL_INTEGRATION, (data) => {
    // `D;<code>` and `A` are the two markers the hook writes. Anything else on this ident belongs
    // to some other producer, so it is reported as unhandled rather than guessed at.
    const [marker, code] = data.split(";");
    // `A` is taken with any payload and not only bare: the fuller form carries the command text,
    // which says nothing this needs, and a marker that means "a command started" means it either way.
    if (marker === "A") {
      onEvent({ kind: "command-started" });
      return true;
    }
    if (marker !== "D" || !/^\d+$/.test(code)) return false;
    onEvent({ kind: "command-finished", exitCode: Number(code) });
    return true;
  });
}
