/**
 * Paths in the terminal that open in the preview on ctrl+click.
 *
 * xterm.js has a link provider for exactly this: it asks, once per hovered line, what that line
 * offers, and underlines whatever comes back. But it has no way to know whether a path names a
 * file, and it draws the underline the moment the mouse arrives. Both are wanted here, so the
 * provider answers asynchronously: it asks the backend whether each candidate is a file this
 * checkout holds and the preview can draw, and only hands xterm the ones that are. A path that
 * does not exist is never underlined, which is the whole behaviour.
 *
 * The cost is one IPC per hovered line rather than per mouse move, because xterm asks per line
 * and stops asking until the line changes. The answers are cached because a terminal prints the
 * same path over and over, and the cache is what keeps a slow disk from making the underline
 * stutter as the mouse travels down a build log.
 *
 * Activation is deliberately narrow: xterm activates a link on any click, so `activate` opens
 * nothing unless ctrl or cmd is held. A plain click on a path stays what it was: a click that
 * selects nothing, which the selection copy already ignores.
 */
import type { IDisposable, ILink, ILinkProvider, IMarker, Terminal } from "@xterm/xterm";
import { probeCheckoutFile } from "./ipc";
import { cellRuns, readLogicalLine } from "./terminal-buffer-line";
import type { CellRun } from "./terminal-buffer-line";
import { terminalPathsIn } from "./terminal-paths";

/**
 * How long a probe's answer is trusted.
 *
 * A build writes the files it is about to mention, so an answer cached for the rest of the
 * session would be wrong for exactly the paths a terminal is most useful on. Long enough that
 * moving the mouse along one line does not re-ask, short enough that a file created while the
 * terminal sits there becomes clickable without a restart.
 */
const PROBE_TTL_MS = 5_000;

/**
 * How many answers are kept.
 *
 * The TTL alone does not bound this: a terminal that prints a thousand distinct paths keeps a
 * thousand entries until each one expires, and the whole map dies with the panel anyway. Oldest
 * goes first, which for a terminal means the paths of the last build rather than the current one.
 */
const MAX_CACHED_PATHS = 256;

export interface FileLinkOptions {
  /**
   * The checkout the terminal belongs to, which is the only tree a path may name.
   *
   * Read through a getter because a terminal keeps its process and this registration when it is
   * moved to another worktree: the tree a path may name is a property of the terminal's current
   * worktree, not of the moment it was opened.
   */
  readonly checkoutId: string;
  /** Opens a confirmed file in the preview. */
  open: (path: string) => void;
}

interface CachedProbe {
  path: string | null;
  at: number;
}

/**
 * The accent a hovered path is drawn in, read from the stylesheet rather than written here, for
 * the same reason the terminal's own colors are: it is a `--marvis-*` token and there are two
 * themes.
 *
 * A decoration takes `#RRGGBB` and nothing else, which is why this is not passed straight
 * through: `color-mix` and the token name both arrive in a form the renderer cannot use. Reading
 * the computed value is also what keeps a theme switch free, because there is no color to
 * re-push.
 */
function accentColor(): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue("--marvis-accent").trim();
  const hex = /^#([\da-f]{6})$/i.exec(value);
  return hex ? `#${hex[1]}` : "";
}

/**
 * Puts the paths in the terminal on screen where ctrl+click opens them.
 *
 * Returns the disposable that takes it back, because a terminal outlives most of what is
 * configured around it: a checkout closing, a panel hiding, a session being replaced.
 */
export function registerFilePathLinks(terminal: Terminal, options: FileLinkOptions): IDisposable {
  const cache = new Map<string, CachedProbe>();
  // One probe per path however many lines are waiting on it: the same path is usually the same
  // path, and asking twice for one answer is one IPC too many on the line the mouse crosses most.
  const inFlight = new Map<string, Promise<string | null>>();
  const paints: { decoration: IDisposable; marker: IMarker }[] = [];
  let disposed = false;

  const clearPaints = () => {
    for (const paint of paints.splice(0)) {
      paint.decoration.dispose();
      paint.marker.dispose();
    }
  };

  const probe = (raw: string): Promise<string | null> => {
    const cached = cache.get(raw);
    if (cached && Date.now() - cached.at < PROBE_TTL_MS) return Promise.resolve(cached.path);
    const pending = inFlight.get(raw);
    if (pending) return pending;

    const request = probeCheckoutFile(options.checkoutId, raw)
      .then((probed) => probed?.path ?? null)
      .catch(() => {
        // A failed probe is a path this cannot open, which is the same answer a missing file gets
        // and the only one a hover can act on. The error belongs to the read the click would do.
        return null;
      })
      .then((path) => {
        inFlight.delete(raw);
        // A terminal that is gone does not need the answer, and a map that outlives it would keep
        // every path the panel ever saw.
        if (disposed) return path;
        if (cache.size >= MAX_CACHED_PATHS) {
          const oldest = cache.keys().next();
          if (!oldest.done) cache.delete(oldest.value);
        }
        cache.set(raw, { path, at: Date.now() });
        return path;
      });
    inFlight.set(raw, request);
    return request;
  };

  /**
   * Draws the hovered path in the accent, which is the one part of the link this app owns.
   *
   * Every run it is handed is drawn, not only the one under the mouse: a path long enough to wrap
   * comes back as one run per row, and underlining the half the pointer happens to be on is what
   * makes a wrapped path look truncated. The whole path is one name, so the whole path is drawn.
   *
   * A decoration takes a 0-based column on the line its marker is on, and the marker is an offset
   * from the cursor rather than a row: both are converted here, because xterm's own 1-based rows
   * and this one disagree in exactly the two places that would silently put the underline on the
   * wrong line.
   */
  const paint = (runs: CellRun[]) => {
    clearPaints();
    const color = accentColor();
    if (!color) return;
    const buffer = terminal.buffer.active;
    // The alternate buffer is a full-screen program's own drawing, and xterm returns no decoration
    // there at all, so asking would only leave a marker nothing is attached to.
    if (buffer.type === "alternate") return;
    for (const run of runs) {
      const offset = run.row - (buffer.baseY + buffer.cursorY);
      const marker = terminal.registerMarker(offset);
      // A row that scrolled out of the buffer takes no marker, and the rest of the path still
      // does: one row it cannot draw is not a reason to draw none of them.
      if (!marker) continue;
      const decoration = terminal.registerDecoration({
        marker,
        x: run.from,
        width: run.to - run.from,
        foregroundColor: color,
      });
      if (!decoration) {
        marker.dispose();
        continue;
      }
      paints.push({ decoration, marker });
    }
  };

  const provider: ILinkProvider = {
    provideLinks(bufferLineNumber, callback) {
      const buffer = terminal.buffer.active;
      // xterm's rows are 1-based and `getLine` is 0-based, so the row it hands over is not the row
      // it can be read from. Reading it as given underlines the line below the one under the mouse.
      const line = readLogicalLine(buffer, bufferLineNumber);
      const candidates = terminalPathsIn(line.text);
      if (candidates.length === 0) {
        callback(undefined);
        return;
      }
      // Every candidate is asked about at once and the slowest answer decides, so a line with
      // three paths underlines in one step instead of three.
      void Promise.all(candidates.map((candidate) => probe(candidate.path))).then((probed) => {
        // A panel that closed mid-probe has nothing left to underline, and answering anyway would
        // draw into a terminal that is being disposed.
        if (disposed) return;
        const links: ILink[] = [];
        for (const [index, candidate] of candidates.entries()) {
          const opened = probed[index];
          // No answer means no link: the path did not survive the check that it is a file this
          // checkout holds, and xterm is not told about it at all, so it cannot underline it.
          if (!opened) continue;
          const runs = cellRuns(line.cells, candidate.start, candidate.end);
          for (const run of runs) {
            links.push({
              // xterm's link ranges are 1-based and the end is the last cell, not the one past it.
              range: {
                start: { x: run.from + 1, y: run.row + 1 },
                end: { x: run.to, y: run.row + 1 },
              },
              // The path the probe confirmed rather than the one the line printed: the confirmed
              // spelling is the checkout-relative one the preview is asked to open.
              text: opened,
              // Underline and pointer are what xterm draws for a link it was given; the accent is
              // this app's own decoration, on top.
              decorations: { pointerCursor: true, underline: true },
              activate(event, text) {
                if (!event.ctrlKey && !event.metaKey) return;
                options.open(text);
              },
              // Every row of the path, not this link's row alone: xterm underlines only the link
              // under the pointer, so the accent is what carries the rest of a wrapped path.
              hover: () => paint(runs),
              leave: clearPaints,
            });
          }
        }
        callback(links.length > 0 ? links : undefined);
      });
    },
  };

  const registration = terminal.registerLinkProvider(provider);
  return {
    dispose: () => {
      disposed = true;
      clearPaints();
      registration.dispose();
    },
  };
}
