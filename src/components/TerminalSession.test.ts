// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";
import { DEFAULT_SETTINGS } from "../domain/settings";
import {
  closeTerminal,
  createTerminal,
  getTerminalStatus,
  loadTerminalLayout,
  moveTerminal,
  resizeTerminal,
  saveTerminalLayout,
  writeTerminal,
} from "../lib/ipc";
import { useToasts } from "../presentation/toasts";
import { theme } from "../presentation/theme";
import SessionPane from "./SessionPane.vue";
import TerminalSession from "./TerminalSession.vue";

const { MockTerminal, terminalMock } = vi.hoisted(() => {
  const terminalMock = {
    channel: null as { onmessage: (buffer: ArrayBuffer) => void } | null,
    input: null as ((value: string) => void) | null,
    resizes: [] as Array<(size: { cols: number; rows: number }) => void>,
    titles: [] as Array<(title: string) => void>,
    scrolls: [] as Array<() => void>,
    output: [] as number[][],
    openCalls: 0,
    focusCalls: 0,
    clearTextureAtlasCalls: 0,
    screenWidth: 0,
    /** The scrollback the mock terminal reports, and the only way a test grows or scrolls it. */
    buffer: { length: 24, viewportY: 0 },
    /** What the component asked xterm to do, in the order it asked. */
    scrollToLines: [] as number[],
    scrollLineCounts: [] as number[],
    events: [] as string[],
    fontLoads: [] as string[],
    fontLoadPromise: null as Promise<FontFace[]> | null,
    /** The size and the scale the terminal was built at, which is the size of its cell from its first frame. */
    builtAt: { fontSize: 16, cursorBlink: true, zoom: 1 } as { fontSize: number; cursorBlink: boolean; zoom: number },
    terminal: null as MockTerminal | null,
  };
  class MockTerminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    element?: HTMLElement;
    constructor() {
      terminalMock.terminal = this;
    }
    loadAddon() {}
    open(element: HTMLElement) {
      this.element = element;
      terminalMock.events.push("open");
      if (terminalMock.screenWidth) element.appendChild(document.createElement("div")).className = "xterm-screen";
      terminalMock.openCalls += 1;
    }
    resize(cols: number, rows: number) {
      this.cols = cols;
      this.rows = rows;
      terminalMock.resizes.forEach((callback) => callback({ cols, rows }));
    }
    onData(callback: (value: string) => void) {
      terminalMock.input = callback;
    }
    onResize(callback: (size: { cols: number; rows: number }) => void) {
      terminalMock.resizes.push(callback);
    }
    onTitleChange(callback: (title: string) => void) {
      terminalMock.titles.push(callback);
      return { dispose: () => {} };
    }
    onScroll(callback: () => void) {
      terminalMock.scrolls.push(callback);
    }
    /** xterm moves the viewport and only then tells anyone, which is what the thumb has to follow. */
    scrollToLine(line: number) {
      terminalMock.scrollToLines.push(line);
      terminalMock.buffer = { ...terminalMock.buffer, viewportY: Math.max(0, line) };
      terminalMock.scrolls.forEach((callback) => callback());
    }
    scrollLines(amount: number) {
      terminalMock.scrollLineCounts.push(amount);
      const viewportY = Math.min(
        Math.max(0, terminalMock.buffer.viewportY + amount),
        Math.max(0, terminalMock.buffer.length - this.rows),
      );
      terminalMock.buffer = { ...terminalMock.buffer, viewportY };
      terminalMock.scrolls.forEach((callback) => callback());
    }
    get buffer() {
      return { active: { ...terminalMock.buffer, baseY: 0, rows: this.rows } };
    }
    write(data: Uint8Array) {
      terminalMock.output.push(Array.from(data));
    }
    focus() {
      terminalMock.focusCalls += 1;
    }
    clearTextureAtlas() {
      terminalMock.clearTextureAtlasCalls += 1;
    }
    registerLinkProvider() {
      return { dispose: () => {} };
    }
    registerMarker() {
      return { dispose: () => {} };
    }
    registerDecoration() {
      return { dispose: () => {} };
    }
    dispose() {}
  }
  return { MockTerminal, terminalMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class MockChannel {
    onmessage: (buffer: ArrayBuffer) => void = () => {};

    constructor() {
      terminalMock.channel = this;
    }
  },
}));

// The terminal's face, palette, addons, ligatures and renderer are the lib's business and are
// asserted there. What this file owns is the wiring: a terminal on the page, with both.
const terminalLib = vi.hoisted(() => ({
  attachTerminalRenderer: vi.fn(),
  setTerminalLigatures: vi.fn(),
  enableTerminalSelectionCopy: vi.fn(() => ({ dispose: vi.fn() })),
  fitCalls: 0,
  /** Stand-in for the palette the lib reads out of the stylesheet, so a switch is visible here. */
  theme: { background: "#282c33" } as Record<string, string>,
}));

// happy-dom has no font loading API. Record the requests so this test can pin that xterm opens only
// after both bundled faces are ready, rather than painting its first atlas from the fallback.
Object.defineProperty(document, "fonts", {
  value: {
    ready: Promise.resolve(),
    load: vi.fn((font: string) => {
      terminalMock.events.push(`font:${font}`);
      terminalMock.fontLoads.push(font);
      return terminalMock.fontLoadPromise ?? Promise.resolve([]);
    }),
  },
  configurable: true,
});

// And no layout either, so every element measures zero and the fit that sizes the grid would never
// run. 80x24 is what a mock terminal of this size already reports, so the dimensions are the ones
// the rest of this file already asserts against.
for (const [property, value] of [
  ["clientWidth", 640],
  ["clientHeight", 480],
] as const) {
  Object.defineProperty(HTMLElement.prototype, property, { value, configurable: true });
}

// The real preload is memoized, so it would only ever load the faces once for the whole file and
// the ordering below would be untestable. Asking for them per mount keeps the guarantee this file
// exists to pin: xterm opens only after both bundled weights are ready.
vi.mock("../lib/marvis-terminal", () => ({
  createMarvisTerminal: (fontSize: number, cursorBlink: boolean, zoom: number) => {
    terminalMock.builtAt = { fontSize, cursorBlink, zoom };
    return new MockTerminal();
  },
  terminalFontSize: (fontSize: number, zoom: number) => fontSize * zoom,
  marvisTerminalTheme: () => terminalLib.theme,
  setTerminalLigatures: terminalLib.setTerminalLigatures,
  enableTerminalSelectionCopy: terminalLib.enableTerminalSelectionCopy,
  attachTerminalRenderer: terminalLib.attachTerminalRenderer,
  preloadTerminalFonts: () =>
    Promise.allSettled([
      document.fonts.load('16px "Marvis Nerd Mono", monospace'),
      document.fonts.load('700 16px "Marvis Nerd Mono", monospace'),
    ]),
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class MockFitAddon {
    fit() {
      terminalLib.fitCalls += 1;
      if (terminalMock.screenWidth) terminalMock.terminal!.cols = 77;
    }
  },
}));

vi.mock("../lib/ipc", () => ({
  closeTerminal: vi.fn(),
  createTerminal: vi.fn(),
  getTerminalStatus: vi.fn(),
  loadTerminalLayout: vi.fn(),
  moveTerminal: vi.fn(),
  resizeTerminal: vi.fn(),
  saveTerminalLayout: vi.fn(),
  writeTerminal: vi.fn(),
}));

const workspace = { repos: [], activeCheckoutId: "checkout:repo", activeSessionId: "session:new" };
const created = {
  session: {
    id: "session:new",
    type: "shell" as const,
    checkoutId: "checkout:repo",
    name: "zsh",
    createdAt: "now",
    status: "inactive" as const,
  },
  workspace,
};

/**
 * Moves the mock's scrollback the way xterm does: the buffer first, then the event, so whatever is
 * listening reads a position that has already moved. The line count is the one thing the tests
 * below depend on being exact — 120 lines of scrollback under 24 rows leaves 96 lines of travel
 * either side of a 480px track, and every number in them is a whole number as a result.
 */
function scrollbackTo(viewportY: number, length = 120) {
  terminalMock.buffer = { length, viewportY };
  terminalMock.scrolls.forEach((callback) => callback());
}

describe("TerminalSession UI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    terminalMock.channel = null;
    terminalMock.input = null;
    terminalMock.resizes = [];
    terminalMock.titles = [];
    terminalMock.scrolls = [];
    terminalMock.output = [];
    terminalMock.openCalls = 0;
    terminalMock.focusCalls = 0;
    terminalMock.clearTextureAtlasCalls = 0;
    terminalMock.screenWidth = 0;
    terminalMock.buffer = { length: 24, viewportY: 0 };
    terminalMock.scrollToLines = [];
    terminalMock.scrollLineCounts = [];
    terminalMock.events = [];
    terminalMock.fontLoads = [];
    terminalMock.fontLoadPromise = null;
    terminalMock.builtAt = { fontSize: 16, cursorBlink: true, zoom: 1 };
    terminalMock.terminal = null;
    terminalLib.fitCalls = 0;
    vi.mocked(createTerminal).mockResolvedValue(created);
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running" });
    vi.mocked(closeTerminal).mockResolvedValue(workspace);
    // `clearAllMocks` empties the calls but keeps an implementation, so a test that leaves this one
    // refusing every size would refuse them for the rest of the file too.
    vi.mocked(resizeTerminal).mockReset();
    vi.mocked(writeTerminal).mockReset();
  });

  it("keeps channel output binary and sends Unicode input and fitted dimensions to the session", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    const output = Uint8Array.of(0, 0xc3, 0xa9, 0x1b, 0x5b, 0x33, 0x31, 0x6d);
    terminalMock.channel?.onmessage(output.buffer as ArrayBuffer);
    terminalMock.input?.("λ pasted");
    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();

    expect(terminalMock.output).toEqual([Array.from(output)]);
    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, expect.anything());
    // Both are refused by xterm.js until the terminal is on the page.
    expect(terminalMock.openCalls).toBe(1);
    expect(terminalLib.setTerminalLigatures).toHaveBeenCalledWith(expect.anything(), true);
    expect(terminalLib.enableTerminalSelectionCopy).toHaveBeenCalledTimes(1);
    expect(terminalLib.attachTerminalRenderer).toHaveBeenCalledTimes(1);
    expect(writeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", new TextEncoder().encode("λ pasted"));
    expect(resizeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", 97, 31);
    wrapper.unmount();
  });

  it("publishes sanitized OSC terminal title changes", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    terminalMock.titles[0]?.(" OpenCode: review task\u001b[31m ");
    expect(wrapper.emitted("statusChanged")?.at(-1)?.[0]).toMatchObject({
      state: "running",
      terminalTitle: "OpenCode: review task[31m",
    });

    terminalMock.titles[0]?.("\u001b[0m");
    expect(wrapper.emitted("statusChanged")?.at(-1)?.[0]).toMatchObject({ terminalTitle: "[0m" });
    terminalMock.titles[0]?.("  ");
    expect(wrapper.emitted("statusChanged")?.at(-1)?.[0]).toMatchObject({ terminalTitle: null });
    wrapper.unmount();
  });

  it("waits for both bundled weights before opening xterm", async () => {
    let releaseFonts!: () => void;
    terminalMock.fontLoadPromise = new Promise<FontFace[]>((resolve) => {
      releaseFonts = () => resolve([]);
    });
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    expect(terminalMock.fontLoads).toEqual([
      '16px "Marvis Nerd Mono", monospace',
      '700 16px "Marvis Nerd Mono", monospace',
    ]);
    expect(terminalMock.openCalls).toBe(0);
    releaseFonts();
    await flushPromises();
    expect(terminalMock.events.at(-1)).toBe("open");
    expect(terminalMock.events.indexOf("open")).toBeGreaterThan(
      terminalMock.events.indexOf('font:700 16px "Marvis Nerd Mono", monospace'),
    );
    expect(terminalMock.clearTextureAtlasCalls).toBe(0);
    wrapper.unmount();
  });

  it("recovers the column FitAddon reserves for a hidden scrollbar without clipping the grid", async () => {
    terminalMock.screenWidth = 732; // 77 columns of 9.5px, rounded by xterm
    const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      const width = this.classList.contains("xterm-screen") ? terminalMock.screenWidth : 747.7;
      return { width } as DOMRect;
    });
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    expect(terminalMock.terminal?.cols).toBe(78); // 78 * 9.5 = 741px fits; 79 would be clipped
    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 78, 24, expect.anything());
    wrapper.unmount();
    bounds.mockRestore();
  });

  it("draws no scrollbar at all in the default hidden mode", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    // Not merely invisible: there is no element. xterm's own scrollbar is already hidden, because
    // it reserves the ten pixels the grid is measured against, and a second one drawn over the
    // edge would be decoration on a terminal that already scrolls with the wheel.
    expect(wrapper.find(".terminal-scrollbar").exists()).toBe(false);
    wrapper.unmount();
  });

  it("sizes the thumb to the scrollback and moves it with the viewport", async () => {
    terminalMock.buffer = { length: 120, viewportY: 0 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();

    // 24 rows of a 480px track with 120 lines behind them: the thumb is a fifth of the track, and
    // the viewport is at the top of the scrollback, so the thumb is too.
    expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);
    expect(wrapper.get(".terminal-scrollbar").attributes()).toMatchObject({
      role: "scrollbar",
      "aria-orientation": "vertical",
      "aria-valuemin": "0",
      "aria-valuemax": "96",
      "aria-valuenow": "0",
      tabindex: "0",
    });
    expect(wrapper.get(".terminal-scrollbar-thumb").attributes("style")).toBe("top: 0px; height: 96px;");

    scrollbackTo(48);
    await flushPromises();

    // Halfway through the scrollback is halfway down the thumb's travel, which is the whole point
    // of deriving one from the other: the thumb is where the viewport is, not near it.
    expect(wrapper.get(".terminal-scrollbar-thumb").attributes("style")).toBe("top: 192px; height: 96px;");
    expect(wrapper.get(".terminal-scrollbar").attributes("aria-valuenow")).toBe("48");
    wrapper.unmount();
  });

  it("leaves `always` showing a full-height thumb when there is nothing to scroll", async () => {
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();

    // The history fits the screen, so there is no position to be anywhere but the only one. A
    // person who asked for a permanent scrollbar is told there is nothing to scroll rather than
    // left wondering whether the setting took.
    expect(wrapper.get(".terminal-scrollbar-thumb").attributes("style")).toBe("top: 0px; height: 480px;");
    wrapper.unmount();
  });

  it("shows the thumb while `auto` is being scrolled and takes it away when nothing moves", async () => {
    vi.useFakeTimers();
    try {
      terminalMock.buffer = { length: 120, viewportY: 0 };
      const wrapper = mount(TerminalSession, {
        props: { checkoutId: "checkout:repo", active: true, scrollbar: "auto" },
      });
      await flushPromises();

      // A pane nobody is scrolling looks like a pane with no scrollbar, which is the reason to
      // choose `auto` over `always` in the first place.
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(true);

      scrollbackTo(48);
      await flushPromises();
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);

      await vi.advanceTimersByTimeAsync(899);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(true);
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not flash a thumb in `auto` for a scrollback that fits on screen", async () => {
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "auto" },
    });
    await flushPromises();

    // 24 lines of buffer under 24 rows is a terminal with no scrollback, which is what every
    // terminal is for the first few seconds of its life. A thumb sized to a track it cannot move
    // along says nothing, so `auto` keeps quiet even when the wheel ticks over it.
    scrollbackTo(0, 24);
    await flushPromises();
    expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(true);
    wrapper.unmount();
  });

  it("keeps the thumb up for as long as `auto` keeps being scrolled", async () => {
    vi.useFakeTimers();
    try {
      terminalMock.buffer = { length: 120, viewportY: 0 };
      const wrapper = mount(TerminalSession, {
        props: { checkoutId: "checkout:repo", active: true, scrollbar: "auto" },
      });
      await flushPromises();

      // Output arriving at the bottom moves the viewport, and the thumb with it. A build that
      // prints for a minute keeps its scrollbar up for that minute, because the thumb really is
      // moving the whole time; the fade is for when the output stops.
      for (let line = 1; line <= 5; line += 1) {
        await vi.advanceTimersByTimeAsync(800);
        scrollbackTo(line * 10);
      }
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);
      await vi.advanceTimersByTimeAsync(900);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(true);
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("scrolls to the line a dragged thumb stands for, and jumps on a press beside it", async () => {
    terminalMock.buffer = { length: 120, viewportY: 0 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();
    const track = wrapper.get(".terminal-scrollbar");

    // Grabbed 24px down a thumb that starts at the top of the track, so the drag keeps the offset
    // it was caught at instead of snapping the thumb's middle under the pointer.
    await track.trigger("pointerdown", { clientY: 24, pointerId: 1 });
    expect(terminalMock.scrollToLines).toEqual([0]);
    await track.trigger("pointermove", { clientY: 480, pointerId: 1 });
    expect(terminalMock.scrollToLines).toEqual([0, 96]);
    expect(wrapper.get(".terminal-scrollbar-thumb").attributes("style")).toBe("top: 384px; height: 96px;");
    await track.trigger("pointerup", { clientY: 480, pointerId: 1 });

    // A press on the empty track puts the middle of the thumb under the pointer instead, which is
    // what every other scrollbar does and what makes clicking above or below the thumb a jump
    // rather than a press that does nothing at all: 200px is 48 above the thumb's middle, so the
    // thumb goes 152px along a 384px travel, which is line 38 of 96.
    await track.trigger("pointerdown", { clientY: 200, pointerId: 2 });
    expect(terminalMock.scrollToLines).toEqual([0, 96, 38]);
    wrapper.unmount();
  });

  it("builds the grid at the scaled cell size and cancels the scale on its own host", async () => {
    // The host is the one thing in the window that is not scaled: the grid is measured and
    // rasterized in the screen's own pixels, so leaving the scale on it would let the compositor
    // stretch a canvas that had already been drawn.
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, fontSize: 20, zoom: 0.8 },
    });
    await flushPromises();

    // The cell is the preference's own size times the window's scale, and it is the scale alone
    // that is cancelled on the host: the size is a real change to the grid, not a transform.
    expect(terminalMock.builtAt).toEqual({ fontSize: 20, cursorBlink: true, zoom: 0.8 });
    expect(wrapper.get(".terminal-host").attributes("style")).toBe("zoom: 1.25;");
    wrapper.unmount();
  });

  it("redraws the grid at the new cell size when the window is scaled, and refits it", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    const fitsBefore = terminalLib.fitCalls;

    await wrapper.setProps({ fontSize: 18, zoom: 1.2 });
    await flushPromises();

    // 18px at 120% is a cell of 21.6, and xterm re-measures and repaints on the assignment; the
    // fit is what tells the PTY how many columns the new cell leaves it.
    expect(terminalMock.terminal?.options).toEqual({ fontSize: 18 * 1.2, cursorBlink: true });
    expect(terminalLib.fitCalls).toBeGreaterThan(fitsBefore);
    expect(wrapper.get(".terminal-host").attributes("style")).toBe("zoom: 0.8333333333333334;");
    wrapper.unmount();
  });

  it("repaints an open terminal in the new palette, and keeps its session", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    const closes = vi.mocked(closeTerminal).mock.calls.length;

    // xterm.js' colors are an object rather than declarations, so the stylesheet cannot reach a
    // terminal on its own: the panel reads the palette again and hands it over. The PTY is not told
    // anything, so nothing is lost to a change of palette.
    terminalLib.theme = { background: "#fafafa" };
    theme.value = "light";
    await flushPromises();

    expect(terminalMock.terminal?.options.theme).toEqual({ background: "#fafafa" });
    expect(vi.mocked(closeTerminal).mock.calls.length).toBe(closes);
    wrapper.unmount();
    theme.value = "dark";
  });

  it("reads a drag on the scaled track as a position in the track's own units", async () => {
    terminalMock.buffer = { length: 120, viewportY: 0 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always", zoom: 1.2 },
    });
    await flushPromises();
    const track = wrapper.get(".terminal-scrollbar");

    // A 480px track at 120% is 576px on the screen, so its middle is at 288 — which is 240 in the
    // track's own units, and line 48 of the 96 lines of travel. Reading the pointer as 288 would
    // have put the thumb on line 60, forty-eight pixels below where it was let go.
    await track.trigger("pointerdown", { clientY: 288, pointerId: 1 });
    expect(terminalMock.scrollToLines).toEqual([48]);
    wrapper.unmount();
  });

  it("keeps `auto` visible during a held drag, then fades after release", async () => {
    vi.useFakeTimers();
    try {
      terminalMock.buffer = { length: 120, viewportY: 0 };
      const wrapper = mount(TerminalSession, {
        props: { checkoutId: "checkout:repo", active: true, scrollbar: "auto" },
      });
      await flushPromises();
      const track = wrapper.get(".terminal-scrollbar");

      await track.trigger("pointerdown", { clientY: 24, pointerId: 1, button: 0 });
      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);

      await track.trigger("pointerup", { clientY: 24, pointerId: 1, button: 0 });
      await vi.advanceTimersByTimeAsync(899);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(true);
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("ignores a move from a pointer that is not the one dragging", async () => {
    terminalMock.buffer = { length: 120, viewportY: 0 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();
    const track = wrapper.get(".terminal-scrollbar");

    await track.trigger("pointerdown", { clientY: 24, pointerId: 1 });
    await track.trigger("pointermove", { clientY: 480, pointerId: 2 });
    expect(terminalMock.scrollToLines).toEqual([0]);
    await track.trigger("pointerup", { clientY: 24, pointerId: 1 });
    await track.trigger("pointermove", { clientY: 480, pointerId: 1 });
    expect(terminalMock.scrollToLines).toEqual([0]);
    wrapper.unmount();
  });

  it("does not drag a thumb on a terminal with no scrollback to drag through", async () => {
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();

    await wrapper.get(".terminal-scrollbar").trigger("pointerdown", { clientY: 240, pointerId: 1 });
    expect(terminalMock.scrollToLines).toEqual([]);
    wrapper.unmount();
  });

  it("preserves wheel delta instead of turning every trackpad tick into a page", async () => {
    terminalMock.buffer = { length: 120, viewportY: 0 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();
    const track = wrapper.get(".terminal-scrollbar");

    // Line-mode wheels keep their line count. Pixel-mode trackpad gestures accumulate to a whole
    // cell, so several small deltas move smoothly instead of each becoming a full-screen jump.
    const lineWheel = new WheelEvent("wheel", {
      deltaY: 3,
      deltaMode: WheelEvent.DOM_DELTA_LINE,
      cancelable: true,
    });
    track.element.dispatchEvent(lineWheel);
    expect(lineWheel.defaultPrevented).toBe(true);
    expect(terminalMock.scrollLineCounts).toEqual([3]);
    track.element.dispatchEvent(
      new WheelEvent("wheel", { deltaY: -3, deltaMode: WheelEvent.DOM_DELTA_LINE, cancelable: true }),
    );
    expect(terminalMock.scrollLineCounts).toEqual([3, -3]);
    track.element.dispatchEvent(
      new WheelEvent("wheel", { deltaY: 10, deltaMode: WheelEvent.DOM_DELTA_PIXEL, cancelable: true }),
    );
    expect(terminalMock.scrollLineCounts).toEqual([3, -3]);
    track.element.dispatchEvent(
      new WheelEvent("wheel", { deltaY: 10, deltaMode: WheelEvent.DOM_DELTA_PIXEL, cancelable: true }),
    );
    expect(terminalMock.scrollLineCounts).toEqual([3, -3, 1]);
    wrapper.unmount();
  });

  it("supports keyboard scrolling on the accessible scrollbar", async () => {
    terminalMock.buffer = { length: 120, viewportY: 48 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, scrollbar: "always" },
    });
    await flushPromises();
    const scrollbar = wrapper.get(".terminal-scrollbar");

    await scrollbar.trigger("keydown", { key: "ArrowDown" });
    await scrollbar.trigger("keydown", { key: "PageUp" });
    await scrollbar.trigger("keydown", { key: "End" });
    expect(terminalMock.scrollToLines).toEqual([49, 26, 96]);
    wrapper.unmount();
  });

  it("picks up the mode chosen after the terminal is already open", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    expect(wrapper.find(".terminal-scrollbar").exists()).toBe(false);

    // The thumb is sized against a track that has to be in the document first, so the switch waits
    // for the element to exist rather than measuring nothing.
    terminalMock.buffer = { length: 120, viewportY: 0 };
    await wrapper.setProps({ scrollbar: "auto" });
    await flushPromises();
    expect(wrapper.get(".terminal-scrollbar-thumb").attributes("style")).toBe("top: 0px; height: 96px;");
    // Turning `auto` on shows it once, so the answer to "did that do anything" is on screen, and
    // the same fade as a scroll is what takes it away again.
    expect(wrapper.find(".terminal-scrollbar-idle").exists()).toBe(false);

    await wrapper.setProps({ scrollbar: "hidden" });
    await flushPromises();
    expect(wrapper.find(".terminal-scrollbar").exists()).toBe(false);
    wrapper.unmount();
  });

  it("requires confirmation before closing a running shell", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    await wrapper.vm.requestClose();
    await flushPromises();
    expect(confirm).toHaveBeenCalledWith(
      "This terminal session is still running. Close the session and stop its process?",
    );
    expect(closeTerminal).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await wrapper.vm.requestClose();
    await flushPromises();
    expect(closeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new");
    expect(wrapper.emitted("closed")).toHaveLength(1);
    wrapper.unmount();
    confirm.mockRestore();
  });

  it("launches a terminal without any prompt, because reviews go to the agent bridge", async () => {
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true },
    });
    await flushPromises();

    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, expect.anything());
    wrapper.unmount();
  });

  it("does not steal focus when a hidden terminal finishes starting", async () => {
    let resolveCreate!: (value: typeof created) => void;
    vi.mocked(createTerminal).mockReturnValue(
      new Promise((resolve) => {
        resolveCreate = resolve;
      }),
    );
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, visible: false, focused: true },
    });
    await flushPromises();
    expect(createTerminal).toHaveBeenCalledTimes(1);
    resolveCreate(created);
    await flushPromises();
    expect(terminalMock.focusCalls).toBe(0);

    await wrapper.setProps({ visible: true, focused: true });
    await flushPromises();
    expect(terminalMock.focusCalls).toBe(1);
    wrapper.unmount();
  });

  it("keeps resize requests scoped to each terminal session", async () => {
    const first = { ...created, session: { ...created.session, id: "session:first" } };
    const second = { ...created, session: { ...created.session, id: "session:second" } };
    vi.mocked(createTerminal).mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    const wrapper = mount({
      components: { TerminalSession },
      template:
        '<div><TerminalSession checkout-id="checkout:repo" :active="true" /><TerminalSession checkout-id="checkout:repo" :active="true" /></div>',
    });
    await flushPromises();

    terminalMock.resizes[0]({ cols: 91, rows: 30 });
    terminalMock.resizes[1]({ cols: 103, rows: 37 });
    await flushPromises();

    expect(resizeTerminal).toHaveBeenCalledWith("checkout:repo", "session:first", 91, 30);
    expect(resizeTerminal).toHaveBeenCalledWith("checkout:repo", "session:second", 103, 37);
    wrapper.unmount();
  });

  it("coalesces resize requests while a terminal resize is pending", async () => {
    let finishFirstResize!: () => void;
    vi.mocked(resizeTerminal).mockImplementationOnce(
      () => new Promise<void>((resolve) => (finishFirstResize = resolve)),
    );
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();
    terminalMock.resizes[0]?.({ cols: 100, rows: 32 });
    terminalMock.resizes[0]?.({ cols: 104, rows: 34 });
    finishFirstResize();
    await flushPromises();

    expect(resizeTerminal).toHaveBeenCalledTimes(2);
    expect(resizeTerminal).toHaveBeenNthCalledWith(1, "checkout:repo", "session:new", 97, 31);
    expect(resizeTerminal).toHaveBeenNthCalledWith(2, "checkout:repo", "session:new", 104, 34);
    wrapper.unmount();
  });

  it("sends a resize that lands while the previous one is still in flight", async () => {
    // The window between a send resolving and the queue draining: the loop has already compared
    // its size against the latest and found them equal, so it breaks, but the chain has not
    // reached the point where new work is accepted again. A measurement arriving in that window is
    // refused by the queue and has to be picked up when it drains, or the shell keeps wrapping to
    // the width the pane had before the layout moved.
    let finishResize!: () => void;
    vi.mocked(resizeTerminal).mockImplementationOnce(() => new Promise<void>((resolve) => (finishResize = resolve)));
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();
    expect(resizeTerminal).toHaveBeenCalledTimes(1);

    finishResize();
    // Inside the window, and not after it: one hop past the comparison, three before the drain.
    await Promise.resolve();
    terminalMock.resizes[0]?.({ cols: 104, rows: 34 });
    await flushPromises();

    expect(resizeTerminal).toHaveBeenNthCalledWith(2, "checkout:repo", "session:new", 104, 34);
    wrapper.unmount();
  });

  it("stops asking once the backend refuses a size", async () => {
    vi.mocked(resizeTerminal).mockRejectedValue(new Error("PTY dimensions must be non-zero"));
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();
    expect(resizeTerminal).toHaveBeenCalledTimes(1);

    // The refused size counts as attempted, so the drain does not ask for it again on its own.
    await flushPromises();
    await flushPromises();
    expect(resizeTerminal).toHaveBeenCalledTimes(1);
    expect(wrapper.get('[role="alert"]').text()).toContain("PTY dimensions must be non-zero");
    wrapper.unmount();
  });

  it("sends no resize after the close has begun, however late it arrives", async () => {
    // The drain re-checks the last measurement as it finishes, so a size that arrives during the
    // close would otherwise be picked up and sent behind the close, against a session the backend
    // is about to shut down.
    let finishResize!: () => void;
    vi.mocked(resizeTerminal).mockImplementationOnce(() => new Promise<void>((resolve) => (finishResize = resolve)));
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running" });

    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();
    expect(resizeTerminal).toHaveBeenCalledTimes(1);

    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const closed = wrapper.vm.requestClose();
    await flushPromises();
    finishResize();
    terminalMock.resizes[0]?.({ cols: 104, rows: 34 });
    await closed;
    await flushPromises();

    expect(resizeTerminal).toHaveBeenCalledTimes(1);
    expect(closeTerminal).toHaveBeenCalledOnce();
    wrapper.unmount();
    confirm.mockRestore();
  });

  it("waits for a resize the drain starts as it finishes before it closes the session", async () => {
    // The close has to be the end of the queue rather than a moment in it: a send still in flight
    // when the PTY goes is a size the session never learns about.
    let finishResize!: () => void;
    vi.mocked(resizeTerminal).mockImplementationOnce(() => new Promise<void>((resolve) => (finishResize = resolve)));
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running" });

    terminalMock.resizes[0]?.({ cols: 97, rows: 31 });
    await flushPromises();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const closed = wrapper.vm.requestClose();
    await flushPromises();
    expect(closeTerminal).not.toHaveBeenCalled();

    finishResize();
    await closed;

    expect(closeTerminal).toHaveBeenCalledOnce();
    wrapper.unmount();
    confirm.mockRestore();
  });

  it("drops a resize that arrives after the pane is gone", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running" });

    wrapper.unmount();
    terminalMock.resizes[0]?.({ cols: 104, rows: 34 });
    await flushPromises();

    expect(resizeTerminal).not.toHaveBeenCalled();
  });

  it("closes the session when its process exits, so an ended shell does not linger", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "exited", exitCode: 0 });
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    // `exit` and `:q` end a shell on purpose, and the panel is where that has to show up. Left
    // open, the session is a frozen last frame that reads as a hung app plus a live-looking entry
    // in the sidebar, and closing it means finding the close button first.
    expect(wrapper.emitted("statusChanged")).toEqual([[{ state: "exited", exitCode: 0 }]]);
    expect(closeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new");
    expect(wrapper.emitted("closed")).toEqual([[workspace]]);
    // The process is already gone, so the question that stopping a live one needs is never asked.
    expect(confirm).not.toHaveBeenCalled();
    wrapper.unmount();
    confirm.mockRestore();
  });

  it("changes the directory of an idle shell, and leaves a busy one alone", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running", foregroundProcess: false });
    const changed = await wrapper.vm.changeDirectory("/work/repo-wt");
    expect(changed).toBe(true);
    expect(writeTerminal).toHaveBeenLastCalledWith(
      "checkout:repo",
      "session:new",
      new TextEncoder().encode("cd '/work/repo-wt'\n"),
    );

    // A build in front of the shell is not told anything: typing `cd` would feed the build.
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running", foregroundProcess: true });
    expect(await wrapper.vm.changeDirectory("/work/repo-wt")).toBe(false);

    // A path is typed into a shell, so anything that could end the line or start a second command
    // is refused rather than quoted into shape.
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running", foregroundProcess: false });
    await wrapper.vm.changeDirectory("/work/repo\nrm -rf /");
    expect(writeTerminal).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("returns false when an input write fails and continues the queue", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running", foregroundProcess: false });
    vi.mocked(writeTerminal).mockRejectedValueOnce({
      code: "terminal_ownership_mismatch",
      message: "terminal session does not belong to the requested checkout",
    });

    expect(await wrapper.vm.changeDirectory("/work/repo-wt")).toBe(false);
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe("terminal session does not belong to the requested checkout");
    expect(await wrapper.vm.changeDirectory("/work/repo-wt")).toBe(true);
    expect(writeTerminal).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("keeps the real PTY error visible after a successful move when the cd write fails", async () => {
    const { toasts, dismiss } = useToasts();
    for (const toast of [...toasts.value]) dismiss(toast.id);
    const checkout: Checkout = {
      id: "checkout:repo",
      repoId: "repo:repo",
      path: "/work/repo",
      canonicalPath: "/work/repo",
      isPrimary: true,
      changedFiles: 0,
      isMissing: false,
      sessions: [],
    };
    const target = { ...checkout, id: "checkout:target", path: "/work/target", isPrimary: false };
    const error = {
      code: "terminal_ownership_mismatch",
      message: "terminal session does not belong to the requested checkout",
    };
    vi.mocked(loadTerminalLayout).mockResolvedValue(null);
    vi.mocked(saveTerminalLayout).mockResolvedValue(undefined);
    vi.mocked(moveTerminal).mockResolvedValue({
      repos: [],
      activeCheckoutId: target.id,
      activeSessionId: created.session.id,
    });

    const wrapper = mount(SessionPane, {
      props: {
        checkout,
        checkouts: [checkout, target],
        activeSessionId: null,
        isOpening: true,
        shellRequest: null,
        terminalSettings: { ...DEFAULT_SETTINGS.terminal, changeDirectoryOnMove: true },
      },
    });
    await wrapper.setProps({
      isOpening: false,
      shellRequest: { checkoutId: checkout.id, token: 1 },
      activeSessionId: created.session.id,
      registeredSessionIds: [created.session.id],
    });
    await flushPromises();
    const terminalSession = wrapper.findComponent(TerminalSession);
    expect(terminalSession.exists()).toBe(true);
    vi.mocked(writeTerminal).mockRejectedValueOnce(error);

    await wrapper.vm.moveSession(created.session.id, target.id);
    await flushPromises();

    expect(moveTerminal).toHaveBeenCalledWith(checkout.id, created.session.id, target.id);
    expect(saveTerminalLayout).toHaveBeenCalledWith(
      target.id,
      expect.objectContaining({ sessionOrder: [created.session.id] }),
    );
    expect(writeTerminal).toHaveBeenCalledWith(
      target.id,
      created.session.id,
      new TextEncoder().encode("cd '/work/target'\n"),
    );
    expect(toasts.value.map((toast) => toast.message)).toEqual([
      target.path + ": the terminal moved, but its directory was not changed.",
    ]);
    expect(toasts.value.map((toast) => toast.message)).not.toContain("busy");
    expect(terminalSession.get('[role="alert"]').text()).toBe(error.message);
    wrapper.unmount();
    for (const toast of [...toasts.value]) dismiss(toast.id);
  });

  it("stops asking for status once the process is gone", async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(getTerminalStatus).mockResolvedValue({ state: "exited", exitCode: 0 });
      const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
      await flushPromises();
      const callsAfterExit = vi.mocked(getTerminalStatus).mock.calls.length;

      await vi.advanceTimersByTimeAsync(5000);

      // A closed session has nothing left to report, and polling it would only produce errors
      // for a session the backend no longer has.
      expect(vi.mocked(getTerminalStatus).mock.calls.length).toBe(callsAfterExit);
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });
});
