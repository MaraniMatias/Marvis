/**
 * The paths a terminal line is offering, found without touching the disk.
 *
 * A terminal prints paths the way a shell hands them over, and the same file appears as
 * `src/lib/x.ts`, `./src/lib/x.ts` and `/Users/me/work/src/lib/x.ts`. It also decorates them:
 * rustc appends `:42:10`, a shell quotes anything with a space in it, and a sentence ends the
 * token in a full stop that is not part of the name. So this is one pass over the line plus the
 * trimming that turns a run of characters into the path a person meant.
 *
 * Nothing here decides whether a file exists: `../` and `/etc/hosts` come out of this just fine
 * and are refused later, by the one component that knows the checkout. Keeping the guess (what
 * does this text look like) apart from the fact (is this openable) is what lets the first be a
 * pure function and the second be a single IPC per hovered line.
 */

/** Characters that close a path in running text but are never part of one. */
const TRAILING_PUNCTUATION = /[.,;:!?)\]}'"`]+$/;

/** Characters that open one: punctuation around the path, not in it. */
const LEADING_PUNCTUATION = /^[[(<'"`]+/;

/** A run of unquoted, non-whitespace characters, which is a path or is nothing. */
const PATH_TOKEN = /[^\s"'`<>]+/g;

/** A quoted run, which is a path even when it has spaces in it. */
const QUOTED_TOKEN = /"([^"]+)"|'([^']+)'/g;

/** `foo.rs:42:10` and `foo.rs:42` are a location, and the file is `foo.rs`. */
const LINE_AND_COLUMN = /:\d+(?::\d+)?$/;

/**
 * Whether a token could name a file, judged by its shape alone.
 *
 * The test is a separator or an extension: a path has a directory part or a dot in its last one.
 * That is what keeps `error` in `error: something broke` from being offered as a file, and what
 * keeps `--verbose` from costing a filesystem lookup on every line the mouse crosses. A file
 * with no extension at all (`Makefile`, `LICENSE`) is not offered, and that is the trade: it
 * is the one file a line mentions that this cannot tell apart from a word.
 */
function looksLikePath(token: string): boolean {
  if (token.length < 3 || token.length > 4096) return false;
  // A URL is a link, not a file: `https://host/a.ts` names nothing on this disk.
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(token)) return false;
  const last = token.slice(token.lastIndexOf("/") + 1);
  return token.includes("/") || last.includes(".");
}

/** One path a line offers, and the characters of the line it spans. */
export interface TerminalPath {
  path: string;
  /** The character the path starts at, which is where the underline begins. */
  start: number;
  /** The character just past the path, which is where it ends. */
  end: number;
}

/**
 * The path a token names, and how far into the token it starts.
 *
 * The offset is only the leading punctuation, because that is the only thing trimmed off the
 * front: what the trailing trimmers and the `:42:10` remove is at the end, and it shortens the
 * token rather than moving where the path begins.
 */
function terminalPathInToken(token: string): { path: string; offset: number } | null {
  const offset = token.length - token.replace(LEADING_PUNCTUATION, "").length;
  const body = token.slice(offset).replace(TRAILING_PUNCTUATION, "");
  // A path that lost everything to the trimmers was never one: `.` and `..` land here.
  if (!body || body === "." || body === "..") return null;
  const path = body.replace(LINE_AND_COLUMN, "");
  if (!path || path === "." || path === "..") return null;
  if (!looksLikePath(path)) return null;
  return { path, offset };
}

/** The path a token names, or `null` when the token names none. */
export function terminalPathIn(token: string): string | null {
  return terminalPathInToken(token)?.path ?? null;
}

/**
 * Every path a line offers, in the order the line prints them, with the characters each covers.
 *
 * The two passes would otherwise find the same path twice, because a quote is not part of the
 * unquoted token rule and so a quoted path reads as an unquoted one as well. Blanking the quoted
 * runs to spaces is what keeps them apart, and spaces are used rather than removal so every
 * column after them still means the same column.
 */
export function terminalPathsIn(line: string): TerminalPath[] {
  const found: TerminalPath[] = [];
  const add = (token: string, at: number) => {
    const parsed = terminalPathInToken(token);
    if (!parsed) return;
    const start = at + parsed.offset;
    found.push({ path: parsed.path, start, end: start + parsed.path.length });
  };
  let unquoted = line;
  for (const match of line.matchAll(QUOTED_TOKEN)) {
    const quoted = match[1] ?? match[2] ?? "";
    add(quoted, match.index + 1);
    unquoted =
      unquoted.slice(0, match.index) + " ".repeat(match[0].length) + unquoted.slice(match.index + match[0].length);
  }
  for (const match of unquoted.matchAll(PATH_TOKEN)) {
    add(match[0], match.index);
  }
  return found.sort((a, b) => a.start - b.start);
}
