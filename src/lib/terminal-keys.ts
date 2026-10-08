/**
 * Whether the program behind a PTY can be told which key was pressed, modifier and all.
 *
 * Shift+Enter is a key of its own, and a terminal that speaks for itself says so: the kitty keyboard
 * protocol encodes it as `ESC [ 13 ; 2 u`, where 13 is the key and 2 is one bit above the base
 * value, which is the shift. Everything else gets `\r`, because that escape is text to anything that
 * has not asked for it — a shell that received it would print the sequence instead of running the
 * line. xterm.js 6 implements no keyboard protocol at all, and its key mapping turns Return into
 * `\r` without ever looking at the shift, so this window has to do the part xterm will not: read
 * what the program announced about its own keyboard, answer it when it asks, and only then answer a
 * modified key with an encoding it can read.
 *
 * A program announces the encoding by pushing the flags it wants (`ESC [ > flags u`), which is also
 * how the older form sets them (`ESC [ = flags ; mode u`). Bit 0 is the one that turns the encoding
 * off, which is why a program that never pushes is a program this leaves alone. An answer to a query
 * (`ESC [ ? flags u`) is deliberately not one of those: it says what this terminal supports, and a
 * program that has decided to read keys this way says so by pushing, not by asking.
 */

/** The one flag that answers "no keys in this encoding", in every form the protocol has. */
const DISABLED_FLAG = 0b1;

/**
 * What this terminal says it supports: every escape code the protocol defines is disambiguated, and
 * nothing is disabled. It is what a query is answered with.
 */
const SUPPORTED_FLAGS = 0;

/**
 * Enough of the previous chunk to hold an announcement the channel boundary cut in half. The longest
 * form the protocol has is a pair of five-digit flags, and a chunk of output ends wherever it ends.
 */
const ANNOUNCEMENT_TAIL = 32;

/** The escape, as a code rather than as a character: written out, it is a control character. */
const ESCAPE = String.fromCharCode(0x1b);

/** A push of the flags a program wants, an answer to a query, or the older form that sets them. */
const ANNOUNCEMENT = new RegExp(`${ESCAPE}\\[([>?=])(\\d+)(?:[;: ]\\d+)*u`, "g");

/** The question a program asks before it decides, which is answered rather than obeyed. */
const QUERY = new RegExp(`${ESCAPE}\\[\\?u`);

export interface KeyboardProtocol {
  /** Whether a modified key can be sent as a key of its own rather than as the bare one it looks like. */
  readonly csiU: boolean;
  /**
   * Reads one chunk of the program's output and keeps what it announced about its keyboard. What
   * goes to the screen belongs to the parser that draws it, and `answer` is how a question is put
   * back to the program that asked it.
   */
  read(output: ArrayBuffer): void;
}

export function watchKeyboardProtocol(answer: (data: string) => void): KeyboardProtocol {
  let enabled = false;
  let tail = "";
  return {
    get csiU() {
      return enabled;
    },
    read(output: ArrayBuffer) {
      // Latin-1 rather than UTF-8: an announcement is ASCII, and decoding would only put the bytes
      // of a character cut in half inside the window. One that ends mid character is harmless here
      // in a way that one ending mid announcement is not.
      const chunk = tail + Array.from(new Uint8Array(output), (byte) => String.fromCharCode(byte)).join("");
      tail = chunk.slice(-ANNOUNCEMENT_TAIL);
      for (const [, form, flags] of chunk.matchAll(ANNOUNCEMENT)) {
        if (form === "?") continue;
        enabled = (Number(flags) & DISABLED_FLAG) === 0;
      }
      if (QUERY.test(chunk)) answer(`${ESCAPE}[?${SUPPORTED_FLAGS}u`);
    },
  };
}
