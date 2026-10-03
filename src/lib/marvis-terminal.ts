import { ClipboardAddon } from "@xterm/addon-clipboard";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import type { ITheme, IDisposable } from "@xterm/xterm";
import { ligatureRanges } from "./ligature-joiner";

/**
 * The faces shared by the terminal and editor and bundled in `src/assets/fonts`, icons included.
 *
 * The icon face is in the list rather than behind the terminal's back because the atlas is
 * rasterized per glyph from this string: a face that is not in it gets a tofu box or a borrowed
 * face drawn at the cell width, and a statusline is mostly separators.
 */
export const TERMINAL_FONT_FAMILY = '"Marvis Nerd Mono", "Marvis Nerd Icons", monospace';

/**
 * A character from the icon face, asked for by name.
 *
 * The face carries nothing but icons, so the load has to say which one or it is never fetched:
 * `document.fonts.load` asks the browser whether a face covers the given text, and with no text it
 * asks about a space, which this face does not have. U+E0B0 is the powerline separator, the glyph
 * drawn on more lines of a terminal than any other icon, and it is the character to keep when the
 * icon set is ever trimmed.
 */
const ICON_PROBE = "\uE0B0";

/**
 * The cell size the terminal draws at, which is the size it is drawn at whether or not the
 * window around it is scaled.
 *
 * The window's scale is a `zoom` on the root element, and the terminal's host cancels it out, so
 * the grid is measured and rasterized at this size in the screen's own pixels. That is the whole
 * reason for the cancel: leaving the host scaled would let the compositor stretch an already drawn
 * WebGL canvas, which is how a terminal ends up legible and soft at the same time.
 */
export function terminalFontSize(fontSize: number, zoom: number): number {
  return fontSize * zoom;
}

/**
 * The terminal's colors, read out of the stylesheet rather than written here (B.1).
 *
 * Every color xterm.js wants is a `--marvis-*` token, so this is a table of what to ask for rather
 * than a palette: the two themes live in `src/marvis.css` and switching between them is a matter
 * of reading the same names again. A name the stylesheet does not answer is left out instead of
 * defaulted here, which leaves xterm.js to draw that one with its own color rather than to paint it
 * with a value nothing else in the app agrees on.
 *
 * The background is in the theme rather than left to CSS because xterm.js paints it as an inline
 * style: the stylesheet has one declaration for the surface and out-specifies that one, rather than
 * pretending the canvas is not painted.
 */
const TERMINAL_THEME_TOKENS = {
  background: "--marvis-content-bg-0",
  foreground: "--marvis-content-text",
  cursor: "--marvis-ansi-bright-white",
  cursorAccent: "--marvis-content-bg-0",
  selectionBackground: "--marvis-selection",
  black: "--marvis-ansi-black",
  red: "--marvis-ansi-red",
  green: "--marvis-ansi-green",
  yellow: "--marvis-ansi-yellow",
  blue: "--marvis-ansi-blue",
  magenta: "--marvis-ansi-magenta",
  cyan: "--marvis-ansi-cyan",
  white: "--marvis-ansi-white",
  brightBlack: "--marvis-ansi-bright-black",
  brightRed: "--marvis-ansi-bright-red",
  brightGreen: "--marvis-ansi-bright-green",
  brightYellow: "--marvis-ansi-bright-yellow",
  brightBlue: "--marvis-ansi-bright-blue",
  brightMagenta: "--marvis-ansi-bright-magenta",
  brightCyan: "--marvis-ansi-bright-cyan",
  brightWhite: "--marvis-ansi-bright-white",
} as const;

/**
 * The palette as it is right now, for a terminal that is already on screen.
 *
 * A theme switch reaches a terminal through this and not through the terminal itself: xterm.js
 * repaints on `options.theme` and keeps its buffer, so the PTY behind it is never told anything and
 * no session is lost to a change of palette.
 */
export function marvisTerminalTheme(): ITheme {
  const computed = getComputedStyle(document.documentElement);
  const theme: ITheme = {};
  for (const [name, token] of Object.entries(TERMINAL_THEME_TOKENS)) {
    const value = computed.getPropertyValue(token).trim();
    // Assigning by name rather than by index: `ITheme` carries properties of other types, so a
    // keyed write is the only one of the three that type-checks against the table above.
    if (value) Object.assign(theme, { [name]: value });
  }
  return theme;
}

/** Which renderer ended up on screen. `dom` is the one that always works. */
export type RendererLevel = "webgl" | "dom";

/**
 * The terminal, configured the way this panel wants it.
 *
 * `allowProposedApi` is on for the ligature joiner and the Unicode provider, which are both
 * experimental in 6.0; it is the price of the two, and it is why this function is the only
 * place a terminal is built.
 *
 * The scale is a parameter because the terminal's cell size has to be the size it is drawn at
 * before it is ever opened: a terminal built at the unscaled cell and scaled afterwards spends
 * its first frame measuring a grid that is already the wrong size. The size and the cursor are
 * preferences from `~/.marvis/config.yml`, read here for the same reason; the ligatures are not,
 * because the joiner only exists once the terminal is on the page.
 *
 * The cursor is a block because that is what a terminal that is not telling you it is waiting for
 * you looks like; whether it blinks is a preference, and a program that asks for another shape
 * through DECSCUSR is answered out of this one rather than fought with it.
 *
 * The block is the palette's bright white and the glyph under it the background, so the cell under
 * the cursor is drawn inverted: that is the swap Konsole makes, and it is what the two tokens above
 * are for. The bright white rather than a white of its own, because the stylesheet already answers
 * that name per theme and a `#ffffff` block on the light surface is a cell nobody can find a cursor
 * in. It is two colors and not one per cell — a cell that arrives with a color of its own does not
 * hand it to the block — and the panel that is not holding the keyboard draws a frame rather than a
 * block, which says so in shape rather than in a color that has to differ from the focused one.
 */
export function createMarvisTerminal(fontSize = 16, cursorBlink = true, zoom = 1): Terminal {
  const terminal = new Terminal({
    allowProposedApi: true,
    cursorBlink,
    cursorStyle: "block",
    cursorInactiveStyle: "outline",
    fontFamily: TERMINAL_FONT_FAMILY,
    fontSize: terminalFontSize(fontSize, zoom),
    lineHeight: 1.2,
    scrollback: 10000,
    theme: marvisTerminalTheme(),
  });
  // Widths and combining marks as Unicode 11 sees them, so emoji and CJK stop breaking the
  // grid that Neovim and the agent TUI draw their panels on. It has to be the active version
  // before the first render, so it cannot wait for `open`.
  terminal.loadAddon(new Unicode11Addon());
  terminal.unicode.activeVersion = "11";
  // xterm.js has no OSC 52 handler of its own, and without one the copy action in Neovim and in
  // the agent TUI does nothing at all.
  terminal.loadAddon(new ClipboardAddon());
  return terminal;
}

let terminalFonts: Promise<unknown> | undefined;

/**
 * All three bundled faces, and the one thing on the path to a drawn terminal that is not the
 * terminal's own work.
 *
 * xterm.js cannot measure its grid against a face it does not have, so a panel that opens
 * before these land draws its first frame from the fallback and lays the columns out wrong.
 * The files are local, but they are not small, and parsing them is the part of opening a
 * terminal a person waits on without being able to see why. The icon face is asked for on its
 * own rather than through the family list above, because a list is not a request: it says where
 * to look once a glyph turns up, and a glyph from the private use range only turns up in a
 * session that has already drawn its first frame at the width the fallback gave it.
 *
 * The request goes out when the app starts rather than when a panel mounts, and the answer is
 * kept: a panel that mounts later awaits a promise that is already settled, so the wait is a
 * microtask instead of a font parse. Callers still await it — the guarantee is that the faces
 * are ready before `open`, not that they were asked for early.
 */
export function preloadTerminalFonts(): Promise<unknown> {
  terminalFonts ??= Promise.allSettled([
    document.fonts.load(`16px ${TERMINAL_FONT_FAMILY}`),
    document.fonts.load(`700 16px ${TERMINAL_FONT_FAMILY}`),
    document.fonts.load(`16px "Marvis Nerd Icons"`, ICON_PROBE),
  ]);
  return terminalFonts;
}

/**
 * The ligatures, which only xterm.js takes once the terminal is on the page: it throws
 * "Terminal must be opened first" before that, and a terminal that cannot be created at all is
 * a far worse outcome than one that draws `=>` as two characters.
 */
function registerLigatures(terminal: Terminal): number {
  try {
    return terminal.registerCharacterJoiner(ligatureRanges);
  } catch {
    // Nothing to announce: the panel still works, it just spells the sequences out. The id is
    // never handed out, so the joiner that was asked for does not exist to be taken back.
    return -1;
  }
}

/** The joiner id each live terminal has, so the preference can be switched and not only set. */
const ligatureJoiners = new WeakMap<Terminal, number>();

/**
 * Turns the ligatures on or off on a terminal that is already on screen.
 *
 * Turning them off deregisters the joiner rather than registering another: xterm.js keeps one at a
 * time, so a second request would silently replace the first and leave nothing to take back. The
 * panel is not recreated for this, which is why the setting reaches the terminals that were
 * already open and not only the next one to be drawn.
 */
export function setTerminalLigatures(terminal: Terminal, enabled: boolean): void {
  if (enabled === ligatureJoiners.has(terminal)) return;
  if (enabled) {
    const joiner = registerLigatures(terminal);
    // A terminal that refused the joiner has none to take back, and remembering a fake one would
    // deregister somebody else's.
    if (joiner >= 0) ligatureJoiners.set(terminal, joiner);
    return;
  }
  terminal.deregisterCharacterJoiner(ligatureJoiners.get(terminal)!);
  ligatureJoiners.delete(terminal);
}

/**
 * Copies whatever a selection gesture leaves selected, the way a text field does: a drag, and a
 * double or triple click, which xterm.js turns into a word or a line. A click that selects nothing
 * copies nothing.
 *
 * It also lets macOS users select text in applications that take over the terminal mouse.
 * Option-drag is xterm.js' force-selection gesture; it is enabled only while an application has
 * mouse tracking active, so the shell keeps its normal rectangular Option selection.
 */
export function enableTerminalSelectionCopy(
  terminal: Terminal,
  copy: (text: string) => void | Promise<void>,
): IDisposable {
  const element = terminal.element;
  if (!element) return { dispose: () => {} };

  const document = element.ownerDocument;
  let dragging = false;
  let moved = false;
  // xterm.js selects the word or the line on the second and third click of a multiple click, and
  // it does it before `mouseup`, so the selection is already there by the time this reads it.
  let multipleClick = false;

  const syncSelectionMode = () => {
    const forceSelection = terminal.modes.mouseTrackingMode !== "none";
    if (terminal.options.macOptionClickForcesSelection !== forceSelection) {
      terminal.options.macOptionClickForcesSelection = forceSelection;
    }
  };
  const onMouseDown = (event: MouseEvent) => {
    if (event.button !== 0) return;
    dragging = true;
    moved = false;
    multipleClick = event.detail > 1;
  };
  const onMouseMove = () => {
    if (dragging) moved = true;
  };
  const onMouseUp = () => {
    if (!dragging) return;
    dragging = false;
    if ((moved || multipleClick) && terminal.hasSelection()) void copy(terminal.getSelection());
  };

  syncSelectionMode();
  element.addEventListener("mousedown", onMouseDown);
  document.addEventListener("mousemove", onMouseMove);
  document.addEventListener("mouseup", onMouseUp);
  const writeParsed = terminal.onWriteParsed(syncSelectionMode);

  return {
    dispose: () => {
      element.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      writeParsed.dispose();
    },
  };
}

/**
 * The renderer, once the terminal is on the page and can be measured.
 *
 * WebGL, because a full-panel TUI repaints on every token and the DOM renderer pays for that in
 * spans. Anything that goes wrong leaves xterm.js' own renderer, which is what the panel drew
 * with before, so the worst case is the behaviour this app already had.
 */
export function attachTerminalRenderer(terminal: Terminal): RendererLevel {
  try {
    // The renderer reads xterm.js internals, so it is pinned to the version it was built
    // against (0.19 with 6.0) and has to move with it.
    const webgl = new WebglAddon();
    // WKWebView drops the context when the machine sleeps or under memory pressure, and letting
    // go of the addon puts the DOM renderer back in its place.
    webgl.onContextLoss(() => webgl.dispose());
    terminal.loadAddon(webgl);
    return "webgl";
  } catch {
    return "dom";
  }
}
