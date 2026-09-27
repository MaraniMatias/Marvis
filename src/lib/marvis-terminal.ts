import { ClipboardAddon } from "@xterm/addon-clipboard";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import type { ITheme } from "@xterm/xterm";
import { ligatureRanges } from "./ligature-joiner";

/**
 * The face is one local Nerd Font, declared under this name in `marvis.css`, so whichever of
 * them is installed is the one the panel draws with. A Nerd Font is not optional here: without
 * it the agent TUI's icons and the code's ligatures have no glyphs at all.
 *
 * There is no "font missing" warning, and there is no way to have an honest one: in WKWebView
 * `document.fonts.check()` answers `false` for this face while it is still unloaded and `true`
 * for a family that does not exist, and `document.fonts.load()` never settles for a family that
 * cannot be found. The only check left would be a timeout race, and a false alarm on a machine
 * that has the font is worse than a quiet fallback.
 */
const TERMINAL_FONT_FAMILY = '"Marvis Nerd Mono", monospace';
const TERMINAL_FONT_SIZE = 16;

/**
 * Only the colors come from the design tokens (B.1); the face and the size are the ones the app
 * has always had. The background is in the theme rather than left to CSS because xterm.js paints
 * it as an inline style: the stylesheet has one declaration for the surface and out-specifies that
 * one, rather than pretending the canvas is not painted.
 *
 * The ANSI entries are there because a colorscheme that uses them would otherwise draw xterm.js'
 * own palette, which has nothing to do with this window. Yellow, magenta and cyan have no token
 * yet: these three are proposals that sit with the rest.
 */
const TERMINAL_THEME: ITheme = {
  background: "#17191f", // --marvis-bg-0
  foreground: "#d6d9e0", // --marvis-text
  cursor: "#7c9eff", // --marvis-accent
  cursorAccent: "#17191f", // --marvis-bg-0
  selectionBackground: "#22252e", // --marvis-selection, the same value ::selection paints with
  black: "#22252e", // --marvis-bg-2
  red: "#e08585", // --marvis-red
  green: "#7fd88f", // --marvis-green
  yellow: "#ddc07f",
  blue: "#7c9eff", // --marvis-accent
  magenta: "#c39ae0",
  cyan: "#7fc9c4",
  white: "#979a9f", // --marvis-text-secondary
  brightBlack: "#5c6072", // --marvis-text-faint
  brightRed: "#ef9d9d",
  brightGreen: "#93e5a2",
  brightYellow: "#ecd08f",
  brightBlue: "#98b2ff",
  brightMagenta: "#d4b0ec",
  brightCyan: "#95dcd8",
  brightWhite: "#d6d9e0", // --marvis-text
};

/** Which renderer ended up on screen. `dom` is the one that always works. */
export type RendererLevel = "webgl" | "dom";

/**
 * The terminal, configured the way this panel wants it.
 *
 * `allowProposedApi` is on for the ligature joiner and the Unicode provider, which are both
 * experimental in 6.0; it is the price of the two, and it is why this function is the only
 * place a terminal is built.
 */
export function createMarvisTerminal(): Terminal {
  const terminal = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    fontFamily: TERMINAL_FONT_FAMILY,
    fontSize: TERMINAL_FONT_SIZE,
    lineHeight: 1.2,
    scrollback: 10000,
    theme: TERMINAL_THEME,
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

/**
 * The ligatures, which only xterm.js takes once the terminal is on the page: it throws
 * "Terminal must be opened first" before that, and a terminal that cannot be created at all is
 * a far worse outcome than one that draws `=>` as two characters.
 */
export function enableTerminalLigatures(terminal: Terminal): void {
  try {
    terminal.registerCharacterJoiner(ligatureRanges);
  } catch {
    // Nothing to announce: the panel still works, it just spells the sequences out.
  }
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
