// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { attachTerminalRenderer, createMarvisTerminal, enableTerminalLigatures } from "./marvis-terminal";

const stubs = vi.hoisted(() => ({
  loaded: [] as string[],
  webglFails: false,
  disposed: 0,
  loseContext: null as (() => void) | null,
  terminal: null as FakeTerminal | null,
}));

class FakeTerminal {
  options: Record<string, unknown>;
  unicode = { activeVersion: "6", versions: ["6", "11"] };
  joiner: ((text: string) => [number, number][]) | null = null;
  addons: FakeAddon[] = [];
  constructor(options: Record<string, unknown>) {
    this.options = options;
    stubs.terminal = this;
  }
  loadAddon(addon: FakeAddon) {
    this.addons.push(addon);
    addon.activate(this);
  }
  registerCharacterJoiner(handler: (text: string) => [number, number][]) {
    this.joiner = handler;
    return 1;
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

  // B.1: the face, size and ligatures are fixed. Only the colors come from the tokens.
  it("builds the terminal the panel has always drawn", () => {
    createMarvisTerminal();

    expect(stubs.terminal?.options).toMatchObject({
      allowProposedApi: true,
      fontFamily: '"Marvis Nerd Mono", monospace',
      fontSize: 16,
      lineHeight: 1.2,
      scrollback: 10000,
      theme: {
        background: "#17191f", // --marvis-bg-0
        foreground: "#d6d9e0", // --marvis-text
        cursor: "#7c9eff", // --marvis-accent
        selectionBackground: "#22252e", // --marvis-selection
      },
    });
  });

  it("gives the terminal a full ANSI palette instead of xterm.js' own", () => {
    const theme = createMarvisTerminal().options.theme as Record<string, string>;

    for (const name of [
      "black",
      "red",
      "green",
      "yellow",
      "blue",
      "magenta",
      "cyan",
      "white",
      "brightBlack",
      "brightWhite",
    ]) {
      expect(theme[name], name).toMatch(/^#[0-9a-f]{6}$/);
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

describe("enableTerminalLigatures", () => {
  it("registers the joiner that draws the programming ligatures", () => {
    const terminal = createMarvisTerminal() as unknown as FakeTerminal;

    enableTerminalLigatures(terminal as never);

    expect(terminal.joiner?.("a => b && c")).toEqual([
      [2, 4],
      [7, 9],
    ]);
  });

  it("does not take the terminal down when xterm.js refuses the joiner", () => {
    const terminal = {
      registerCharacterJoiner: () => {
        throw new Error("Terminal must be opened first");
      },
    };

    expect(() => enableTerminalLigatures(terminal as never)).not.toThrow();
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
