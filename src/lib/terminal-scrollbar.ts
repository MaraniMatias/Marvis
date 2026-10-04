/**
 * Where the thumb of Marvis' own terminal scrollbar goes.
 *
 * xterm already draws a scrollbar, and this app hides it: the scrollable element that carries it
 * sits in the middle of the pane and the fit measures the grid against that box, so a scrollbar
 * that reserves its 10px is 10px the terminal has to give back as columns. The rules that hide it
 * are in `style.css` and the reason they exist is written there. What is left is a terminal that
 * can be scrolled (by wheel, by keyboard, by dragging a selection past the top) with nothing on
 * screen to say how much scrollback there is or where in it you are.
 *
 * So the scrollbar is drawn here instead, as an overlay that covers the right edge without
 * participating in layout. It cannot cost a column, it is not xterm's element to fight with, and
 * its geometry is this file's whole job: a function over four numbers, with no DOM in it, so the
 * arithmetic can be read and tested without a terminal attached.
 *
 * The maths is the scrollbar every desktop has had for thirty years. The thumb is as much of the
 * track as the viewport is of the buffer, never smaller than a thumb you can catch, and its
 * position along the rest of the track is the fraction of the scrollback that is behind you: the
 * same fraction, which is why dragging it moves the viewport by the right number of lines.
 */

/** A thumb thinner than this cannot be grabbed with a finger or aimed with a mouse. */
const MIN_THUMB_PX = 28;

export interface TerminalScrollback {
  /** Every line the scrollback holds, the visible rows included. */
  length: number;
  /** The line the viewport starts at, counted from the top of the scrollback. */
  viewportY: number;
  /** How many lines the viewport shows. */
  rows: number;
}

export interface TerminalScrollbarGeometry {
  /** Where the top of the thumb sits, in px from the top of the track. */
  top: number;
  /** How tall the thumb is, in px. Never more than the track. */
  height: number;
  /** Whether there is anything to scroll, which is also whether a thumb is worth drawing. */
  scrollable: boolean;
  /** The furthest the viewport can start from the top of the scrollback, in lines. */
  maxOffset: number;
  /** How far the thumb itself can travel down the track, in px. Zero when it fills the track. */
  travel: number;
}

/**
 * The thumb for a viewport somewhere in a scrollback, drawn on a track of a known height.
 *
 * Every number is clamped rather than trusted, because the buffer is the one thing here that is
 * measured by a program that may be mid-write: `length` can be shorter than the rows on the frame
 * a terminal is resized on, and a viewport past the end of a buffer that has just been trimmed
 * would otherwise throw the thumb off the bottom of its own track.
 */
export function terminalScrollbarGeometry(
  scrollback: TerminalScrollback,
  trackHeight: number,
): TerminalScrollbarGeometry {
  const rows = Math.max(1, Math.floor(scrollback.rows));
  const length = Math.max(rows, Math.floor(scrollback.length));
  const maxOffset = Math.max(0, length - rows);
  const viewportY = Math.min(maxOffset, Math.max(0, Math.floor(scrollback.viewportY)));
  const track = Math.max(0, Math.floor(trackHeight));
  if (track === 0) return { top: 0, height: 0, scrollable: maxOffset > 0, maxOffset, travel: 0 };
  if (maxOffset === 0) return { top: 0, height: track, scrollable: false, maxOffset: 0, travel: 0 };
  const height = Math.min(track, Math.max(MIN_THUMB_PX, (rows / length) * track));
  const travel = track - height;
  return { top: (viewportY / maxOffset) * travel, height, scrollable: true, maxOffset, travel };
}

/**
 * The scroll position a thumb dropped at `top` stands for, which is the inverse of the arithmetic
 * above and the only thing a drag needs: the pointer says where the thumb is, the buffer wants a
 * line, and rounding to a whole line is what stops the thumb from chasing a sub-pixel remainder.
 *
 * A track with no travel has one position, so it answers 0 rather than a division by zero.
 */
export function scrollbarOffsetForTop(top: number, geometry: TerminalScrollbarGeometry): number {
  if (geometry.travel <= 0) return 0;
  const clamped = Math.min(geometry.travel, Math.max(0, top));
  return Math.round((clamped / geometry.travel) * geometry.maxOffset);
}
