/**
 * The paths and the web links a terminal line is offering, found without touching the disk.
 *
 * A terminal prints paths the way a shell hands them over, and the same file appears as
 * `src/lib/x.ts`, `./src/lib/x.ts` and `/Users/me/work/src/lib/x.ts`. It also decorates them:
 * rustc appends `:42:10`, a shell quotes anything with a space in it, and a sentence ends the
 * token in a full stop that is not part of the name. So this is one pass over the line plus the
 * trimming that turns a run of characters into the path a person meant.
 *
 * A URL is offered here too, and it is offered as what it is rather than as a path: `https://…`
 * names a page, not a file in this checkout, so it goes to the browser instead of the preview and
 * is never asked about on disk. The two are kept apart by one rule each — a path needs a
 * separator or an extension and refuses a scheme, a link needs an `http` or `https` one — which is
 * the same pair `DocumentPane` and the backend's opener already decide on.
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
 * A page, and the only thing the browser is asked to open.
 *
 * `http` or `https` and nothing else, with no whitespace inside: the same pair the preview's own
 * links go by and the same one `opener.rs` refuses everything outside of. So a token that is not
 * one here is a token nothing could have opened anyway, and it is not underlined.
 */
const WEB_URL = /^https?:\/\/\S+$/i;

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

/** One web link a line offers, and the characters of the line it spans. */
export interface TerminalUrl {
  url: string;
  /** The character the url starts at, which is where the underline begins. */
  start: number;
  /** The character just past the url, which is where it ends. */
  end: number;
}

/** A run of the line a token rule matched, and the character that run starts at. */
interface TokenSpan {
  token: string;
  /** The character of the line the token begins at, opening quote included and excluded. */
  offset: number;
}

/**
 * Every run of the line a path or a URL could be, quoted ones first and then the unquoted ones.
 *
 * The two passes would otherwise find the same run twice, because a quote is not part of the
 * unquoted token rule and so a quoted path reads as an unquoted one as well. Blanking the quoted
 * runs to spaces is what keeps them apart, and spaces are used rather than removal so every
 * column after them still means the same column.
 *
 * Which passes there are, and which token answers for what, is the caller's: the shape is the
 * only thing shared, because a URL and a path are trimmed and tested differently from here on.
 */
function tokenSpans(line: string): TokenSpan[] {
  const spans: TokenSpan[] = [];
  let unquoted = line;
  for (const match of line.matchAll(QUOTED_TOKEN)) {
    spans.push({ token: match[1] ?? match[2] ?? "", offset: match.index + 1 });
    unquoted =
      unquoted.slice(0, match.index) + " ".repeat(match[0].length) + unquoted.slice(match.index + match[0].length);
  }
  for (const match of unquoted.matchAll(PATH_TOKEN)) {
    spans.push({ token: match[0], offset: match.index });
  }
  return spans;
}

/**
 * The token without the punctuation a sentence put around it, and how far into the token that is.
 *
 * The offset is only the leading punctuation, because that is the only thing trimmed off the
 * front: what the trailing trimmer removes is at the end, and it shortens the token rather than
 * moving where the name begins.
 */
function trimmedToken(token: string): { body: string; offset: number } | null {
  const offset = token.length - token.replace(LEADING_PUNCTUATION, "").length;
  const body = token.slice(offset).replace(TRAILING_PUNCTUATION, "");
  return body ? { body, offset } : null;
}

/**
 * The path a token names, and how far into the token it starts.
 *
 * The offset is only the leading punctuation, because that is the only thing trimmed off the
 * front: what the trailing trimmers and the `:42:10` remove is at the end, and it shortens the
 * token rather than moving where the path begins.
 */
function terminalPathInToken(token: string): { path: string; offset: number } | null {
  const trimmed = trimmedToken(token);
  if (!trimmed) return null;
  // A path that lost everything to the trimmers was never one: `.` and `..` land here.
  if (trimmed.body === "." || trimmed.body === "..") return null;
  const path = trimmed.body.replace(LINE_AND_COLUMN, "");
  if (!path || path === "." || path === "..") return null;
  if (!looksLikePath(path)) return null;
  return { path, offset: trimmed.offset };
}

/**
 * Whether a run still holds an `(` that the trailing trimmer closed on its behalf.
 *
 * A parenthesis is legal inside a URL and common in one — every Wikipedia and MDN article with a
 * qualifier in its title carries one — so trimming it off hands back an address that is not the
 * one on screen, and the browser opens a page that does not exist. A close with nothing open in
 * front of it is the sentence's, and stays trimmed.
 */
function hasUnclosedParenthesis(body: string): boolean {
  let open = 0;
  for (const character of body) {
    if (character === "(") open += 1;
    else if (character === ")") open -= 1;
  }
  return open > 0;
}

/** The page a token names, and how far into the token it starts, or `null` when it names none. */
function terminalUrlInToken(token: string): { url: string; offset: number } | null {
  const trimmed = trimmedToken(token);
  if (!trimmed) return null;
  // The token ending in `)` is what says a close was trimmed: a body that still had one would not
  // be here to test. Restoring it is what keeps `…/Terminal_emulator_(OS)` a page that exists.
  const url = token.endsWith(")") && hasUnclosedParenthesis(trimmed.body) ? `${trimmed.body})` : trimmed.body;
  if (!WEB_URL.test(url)) return null;
  return { url, offset: trimmed.offset };
}

/** The path a token names, or `null` when the token names none. */
export function terminalPathIn(token: string): string | null {
  return terminalPathInToken(token)?.path ?? null;
}

/**
 * Every path a line offers, in the order the line prints them, with the characters each covers.
 *
 * A token that names a page rather than a file is not one of these, and `terminalUrlsIn` is where
 * it goes instead.
 */
export function terminalPathsIn(line: string): TerminalPath[] {
  const found: TerminalPath[] = [];
  for (const span of tokenSpans(line)) {
    const parsed = terminalPathInToken(span.token);
    if (!parsed) continue;
    const start = span.offset + parsed.offset;
    found.push({ path: parsed.path, start, end: start + parsed.path.length });
  }
  return found.sort((a, b) => a.start - b.start);
}

/**
 * Every page a line offers, in the order the line prints them, with the characters each covers.
 *
 * Nothing is asked about these and nothing is confirmed: a URL is its own evidence, because the
 * address on screen is the address the browser is asked for. So this is one pass over the line
 * with no waiting on the other end, which is why the link it finds can be drawn on the same tick
 * the mouse arrives rather than after a probe.
 */
export function terminalUrlsIn(line: string): TerminalUrl[] {
  const found: TerminalUrl[] = [];
  for (const span of tokenSpans(line)) {
    const parsed = terminalUrlInToken(span.token);
    if (!parsed) continue;
    const start = span.offset + parsed.offset;
    found.push({ url: parsed.url, start, end: start + parsed.url.length });
  }
  return found.sort((a, b) => a.start - b.start);
}
