// @vitest-environment happy-dom
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  attachTerminalRenderer,
  createMarvisTerminal,
  enableTerminalSelectionCopy,
  marvisTerminalTheme,
  setTerminalLigatures,
} from "./marvis-terminal";

/**
 * The token table as it is written down. Read from disk rather than imported, because vitest hands
 * back an empty string for a stylesheet it does not run, and an empty table would make every
 * assertion below pass for the wrong reason.
 */
const stylesheet = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "marvis.css"), "utf8");

const stubs = vi.hoisted(() => ({
  loaded: [] as string[],
  fontLoads: [] as string[],
  webglFails: false,
  disposed: 0,
  loseContext: null as (() => void) | null,
  terminal: null as FakeTerminal | null,
}));

class FakeTerminal {
  options: Record<string, unknown>;
  unicode = { activeVersion: "6", versions: ["6", "11"] };
  element = document.createElement("div");
  modes = { mouseTrackingMode: "none" as "none" | "drag" };
  selection = "";
  writeParsedListeners: Array<() => void> = [];
  addons: FakeAddon[] = [];
  constructor(options: Record<string, unknown>) {
    this.options = options;
    stubs.terminal = this;
  }
  loadAddon(addon: FakeAddon) {
    this.addons.push(addon);
    addon.activate(this);
  }
  joiners = new Map<number, (text: string) => [number, number][]>();
  joinerFailures = false;
  nextJoinerId = 1;
  registerCharacterJoiner(handler: (text: string) => [number, number][]) {
    if (this.joinerFailures) throw new Error("Terminal must be opened first");
    const id = this.nextJoinerId++;
    this.joiners.set(id, handler);
    return id;
  }
  deregisterCharacterJoiner(id: number) {
    this.joiners.delete(id);
  }
  get joiner() {
    return [...this.joiners.values()][0] ?? null;
  }
  set joiner(_handler: ((text: string) => [number, number][]) | null) {
    // The fake kept a single joiner before the preference became switchable; the setter is kept so
    // nothing has to know that, and it is the last registered one xterm.js would draw.
  }
  onWriteParsed(listener: () => void) {
    this.writeParsedListeners.push(listener);
    return {
      dispose: () => (this.writeParsedListeners = this.writeParsedListeners.filter((item) => item !== listener)),
    };
  }
  hasSelection() {
    return Boolean(this.selection);
  }
  getSelection() {
    return this.selection;
  }
}

interface FakeAddon {
  name: string;
  activate(terminal: FakeTerminal): void;
  dispose?(): void;
}

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    constructor(options: Record<string, unknown>) {
      stubs.terminal = new FakeTerminal(options);
      return stubs.terminal as never;
    }
  },
}));

vi.mock("@xterm/addon-unicode11", () => ({
  Unicode11Addon: class implements FakeAddon {
    name = "unicode11";
    activate(terminal: FakeTerminal) {
      terminal.unicode.versions.push("11");
      stubs.loaded.push("unicode11");
    }
  },
}));

vi.mock("@xterm/addon-clipboard", () => ({
  ClipboardAddon: class implements FakeAddon {
    name = "clipboard";
    activate() {
      stubs.loaded.push("clipboard");
    }
  },
}));

vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class implements FakeAddon {
    name = "webgl";
    private _listener: (() => void) | null = null;
    onContextLoss(listener: () => void) {
      this._listener = listener;
    }
    activate() {
      if (stubs.webglFails) throw new Error("WebGL2 is not supported");
      stubs.loaded.push("webgl");
      stubs.loseContext = () => this._listener?.();
    }
    dispose() {
      stubs.disposed += 1;
    }
  },
}));

function fakeTerminal() {
  return new FakeTerminal({});
}

describe("createMarvisTerminal", () => {
  beforeEach(() => {
    stubs.loaded = [];
    stubs.webglFails = false;
    stubs.disposed = 0;
    stubs.loseContext = null;
  });

  // B.1: the face and the colors come from the tokens; the size and the cursor are preferences.
  it("builds the terminal the panel has always drawn", () => {
    createMarvisTerminal();

    expect(stubs.terminal?.options).toMatchObject({
      allowProposedApi: true,
      cursorBlink: true,
      // A block is what a terminal looks like when it is not asking you to wait for it.
      cursorStyle: "block",
      fontFamily: '"Marvis Nerd Mono", monospace',
      fontSize: 16,
      lineHeight: 1.2,
      scrollback: 10000,
    });
  });

  it("paints itself out of the stylesheet rather than out of a palette of its own", () => {
    // Every color is a `--marvis-*` token, so which one arrives is the theme's answer rather than
    // this file's: a terminal opened in the light palette gets the light one without a second copy
    // of the values anywhere.
    const style = document.createElement("style");
    style.textContent = `:root { --marvis-bg-0: #282c33; --marvis-text: #dce0e5; }
      :root[data-theme="light"] { --marvis-bg-0: #fafafa; --marvis-text: #242529; }`;
    document.head.append(style);
    try {
      document.documentElement.dataset.theme = "dark";
      expect(marvisTerminalTheme()).toMatchObject({ background: "#282c33", foreground: "#dce0e5" });

      document.documentElement.dataset.theme = "light";
      expect(marvisTerminalTheme()).toMatchObject({ background: "#fafafa", foreground: "#242529" });
    } finally {
      style.remove();
    }
  });

  it("builds the terminal at the size and cursor the settings ask for", () => {
    createMarvisTerminal(20, false, 1.2);

    // The size is the preference's own, multiplied by the window's scale: the terminal's host
    // cancels the scale out so the grid is measured in screen pixels, which means the cell has to
    // be handed the scaled size rather than inheriting one.
    expect(stubs.terminal?.options).toMatchObject({ fontSize: 24, cursorBlink: false });
  });

  it("hands xterm.js a full ANSI palette, and every color of it out of the stylesheet", () => {
    // The names are read out of the real stylesheet rather than listed here, because the list that
    // matters is the one `marvis.css` carries: a token renamed there has to take this down with it,
    // and xterm.js' own palette — which is not this window's — is what a missing one falls back to.
    const names = [...new Set([...stylesheet.matchAll(/(--marvis-[a-z0-9-]+):/g)].map(([, name]) => name))];
    const colors = names.map((_, index) => `#${(index + 1).toString(16).padStart(6, "0")}`);
    const style = document.createElement("style");
    style.textContent = `:root { ${names.map((name, index) => `${name}: ${colors[index]};`).join(" ")} }`;
    document.head.append(style);
    try {
      const theme = marvisTerminalTheme();

      expect(Object.keys(theme).sort()).toEqual([
        "background",
        "black",
        "blue",
        "brightBlack",
        "brightBlue",
        "brightCyan",
        "brightGreen",
        "brightMagenta",
        "brightRed",
        "brightWhite",
        "brightYellow",
        "cursor",
        "cursorAccent",
        "cyan",
        "foreground",
        "green",
        "magenta",
        "red",
        "selectionBackground",
        "white",
        "yellow",
      ]);
      for (const value of Object.values(theme)) expect(colors).toContain(value);
    } finally {
      style.remove();
    }
  });

  it("turns on Unicode 11 and the clipboard handler before the first render", () => {
    const terminal = createMarvisTerminal() as unknown as FakeTerminal;

    // Both are proposed API in 6.0, and both change how the terminal measures itself or answers
    // a paste, so they have to be in place before it is on the page.
    expect(terminal.unicode.activeVersion).toBe("11");
    expect(stubs.loaded).toEqual(["unicode11", "clipboard"]);
  });
});

describe("setTerminalLigatures", () => {
  it("registers the joiner that draws the programming ligatures", () => {
    const terminal = createMarvisTerminal() as unknown as FakeTerminal;

    setTerminalLigatures(terminal as never, true);

    expect(terminal.joiner?.("a => b && c")).toEqual([
      [2, 4],
      [7, 9],
    ]);
  });

  it("takes the joiner back when the preference is switched off, and puts it back", () => {
    const terminal = createMarvisTerminal() as unknown as FakeTerminal;

    setTerminalLigatures(terminal as never, true);
    setTerminalLigatures(terminal as never, false);
    expect(terminal.joiner).toBeNull();

    setTerminalLigatures(terminal as never, true);
    expect(terminal.joiner?.("a => b")).toEqual([[2, 4]]);
    // Switching to the value the terminal is already on is a no-op, so a preference that is
    // re-applied does not stack joiners.
    setTerminalLigatures(terminal as never, true);
    expect(terminal.joiners.size).toBe(1);
  });

  it("does not take the terminal down when xterm.js refuses the joiner", () => {
    const terminal = createMarvisTerminal() as unknown as FakeTerminal;
    terminal.joinerFailures = true;

    expect(() => setTerminalLigatures(terminal as never, true)).not.toThrow();
    // Nothing was registered, so nothing has to be taken back, and switching off is still safe.
    expect(() => setTerminalLigatures(terminal as never, false)).not.toThrow();
  });
});

describe("enableTerminalSelectionCopy", () => {
  it("copies a selection after a mouse drag", () => {
    const terminal = fakeTerminal();
    terminal.selection = "selected text";
    const copy = vi.fn();
    const disposable = enableTerminalSelectionCopy(terminal as never, copy);

    terminal.element.dispatchEvent(new MouseEvent("mousedown", { button: 0 }));
    document.dispatchEvent(new MouseEvent("mousemove", { buttons: 1 }));
    document.dispatchEvent(new MouseEvent("mouseup", { button: 0 }));

    expect(copy).toHaveBeenCalledWith("selected text");
    disposable.dispose();
  });

  // A double click selects the word xterm.js measured, so it copies like a drag does.
  it("copies the word or the line a double and a triple click select", () => {
    const terminal = fakeTerminal();
    const copy = vi.fn();
    const disposable = enableTerminalSelectionCopy(terminal as never, copy);

    for (const detail of [2, 3]) {
      terminal.selection = `click ${detail}`;
      terminal.element.dispatchEvent(new MouseEvent("mousedown", { button: 0, detail }));
      document.dispatchEvent(new MouseEvent("mouseup", { button: 0, detail }));
    }

    expect(copy).toHaveBeenNthCalledWith(1, "click 2");
    expect(copy).toHaveBeenNthCalledWith(2, "click 3");
    disposable.dispose();
  });

  it("does not copy a click or a drag without a selection", () => {
    const terminal = fakeTerminal();
    const copy = vi.fn();
    const disposable = enableTerminalSelectionCopy(terminal as never, copy);

    terminal.element.dispatchEvent(new MouseEvent("mousedown", { button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { button: 0 }));
    terminal.element.dispatchEvent(new MouseEvent("mousedown", { button: 0 }));
    document.dispatchEvent(new MouseEvent("mousemove", { buttons: 1 }));
    document.dispatchEvent(new MouseEvent("mouseup", { button: 0 }));
    terminal.element.dispatchEvent(new MouseEvent("mousedown", { button: 0, detail: 2 }));
    document.dispatchEvent(new MouseEvent("mouseup", { button: 0, detail: 2 }));

    expect(copy).not.toHaveBeenCalled();
    disposable.dispose();
  });

  it("forces macOS selection only while an application owns the mouse", () => {
    const terminal = fakeTerminal();
    const disposable = enableTerminalSelectionCopy(terminal as never, vi.fn());

    expect(terminal.options.macOptionClickForcesSelection).toBe(false);
    terminal.modes.mouseTrackingMode = "drag";
    terminal.writeParsedListeners.forEach((listener) => listener());
    expect(terminal.options.macOptionClickForcesSelection).toBe(true);
    terminal.modes.mouseTrackingMode = "none";
    terminal.writeParsedListeners.forEach((listener) => listener());
    expect(terminal.options.macOptionClickForcesSelection).toBe(false);
    disposable.dispose();
  });
});

describe("attachTerminalRenderer", () => {
  beforeEach(() => {
    stubs.loaded = [];
    stubs.webglFails = false;
    stubs.disposed = 0;
    stubs.loseContext = null;
  });

  it("draws with WebGL, because a TUI repaints the whole panel", () => {
    const terminal = fakeTerminal();

    expect(attachTerminalRenderer(terminal as never)).toBe("webgl");
    expect(stubs.loaded).toEqual(["webgl"]);
  });

  it("keeps the DOM renderer when WebGL2 is not there", () => {
    stubs.webglFails = true;

    expect(attachTerminalRenderer(fakeTerminal() as never)).toBe("dom");
    expect(stubs.loaded).toEqual([]);
  });

  it("lets go of the renderer when the context is lost, so the terminal keeps drawing", () => {
    const terminal = fakeTerminal();
    attachTerminalRenderer(terminal as never);

    stubs.loseContext?.();

    expect(stubs.disposed).toBe(1);
  });
});

describe("preloadTerminalFonts", () => {
  // The kept promise is module state on purpose, so each of these needs the module fresh or the
  // first test's answer would be handed to the second. happy-dom has no font loading API either,
  // so `document.fonts` stands in for it.
  async function freshPreload(load: (font: string) => Promise<unknown>) {
    vi.resetModules();
    stubs.fontLoads = [];
    Object.defineProperty(document, "fonts", {
      value: {
        load: vi.fn((font: string) => {
          stubs.fontLoads.push(font);
          return load(font);
        }),
      },
      configurable: true,
    });
    const { preloadTerminalFonts } = await import("./marvis-terminal");
    return preloadTerminalFonts();
  }

  // The whole point of preloading is that the second caller pays nothing, which only holds if
  // the answer is kept. Without this, every panel re-parses 4.8MB of TTF on its own mount.
  it("asks for both weights once and hands every later caller the same promise", async () => {
    await freshPreload(() => Promise.resolve([]));
    const { preloadTerminalFonts } = await import("./marvis-terminal");
    const first = preloadTerminalFonts();
    const second = preloadTerminalFonts();

    expect(second).toBe(first);
    expect(stubs.fontLoads).toEqual(['16px "Marvis Nerd Mono", monospace', '700 16px "Marvis Nerd Mono", monospace']);
    await expect(first).resolves.toHaveLength(2);
  });

  it("resolves rather than rejects when a face is missing, so a panel still opens", async () => {
    await expect(freshPreload(() => Promise.reject(new Error("no such face")))).resolves.toHaveLength(2);
  });
});
