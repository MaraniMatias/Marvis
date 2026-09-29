// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import TerminalSession from "./TerminalSession.vue";

const { MockTerminal, terminalMock } = vi.hoisted(() => {
  const terminalMock = {
    channel: null as { onmessage: (buffer: ArrayBuffer) => void } | null,
    input: null as ((value: string) => void) | null,
    resizes: [] as Array<(size: { cols: number; rows: number }) => void>,
    output: [] as number[][],
    openCalls: 0,
    focusCalls: 0,
    clearTextureAtlasCalls: 0,
    screenWidth: 0,
    terminal: null as MockTerminal | null,
  };
  class MockTerminal {
    cols = 80;
    rows = 24;
    options = {};
    element?: HTMLElement;
    constructor() {
      terminalMock.terminal = this;
    }
    loadAddon() {}
    open(element: HTMLElement) {
      this.element = element;
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
    write(data: Uint8Array) {
      terminalMock.output.push(Array.from(data));
    }
    focus() {
      terminalMock.focusCalls += 1;
    }
    clearTextureAtlas() {
      terminalMock.clearTextureAtlasCalls += 1;
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
  enableTerminalLigatures: vi.fn(),
  enableTerminalSelectionCopy: vi.fn(() => ({ dispose: vi.fn() })),
  fitCalls: 0,
}));

// happy-dom has no font loading API, and the panel waits on it before fitting a second time. A
// browser that has already resolved the face resolves this immediately, which is what this is.
Object.defineProperty(document, "fonts", { value: { ready: Promise.resolve() }, configurable: true });

// And no layout either, so every element measures zero and the fit that sizes the grid would never
// run. 80x24 is what a mock terminal of this size already reports, so the dimensions are the ones
// the rest of this file already asserts against.
for (const [property, value] of [
  ["clientWidth", 640],
  ["clientHeight", 480],
] as const) {
  Object.defineProperty(HTMLElement.prototype, property, { value, configurable: true });
}

vi.mock("../lib/marvis-terminal", () => ({
  createMarvisTerminal: () => new MockTerminal(),
  enableTerminalLigatures: terminalLib.enableTerminalLigatures,
  enableTerminalSelectionCopy: terminalLib.enableTerminalSelectionCopy,
  attachTerminalRenderer: terminalLib.attachTerminalRenderer,
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
  resizeTerminal: vi.fn(),
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

describe("TerminalSession UI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    terminalMock.channel = null;
    terminalMock.input = null;
    terminalMock.resizes = [];
    terminalMock.output = [];
    terminalMock.openCalls = 0;
    terminalMock.focusCalls = 0;
    terminalMock.clearTextureAtlasCalls = 0;
    terminalMock.screenWidth = 0;
    terminalMock.terminal = null;
    terminalLib.fitCalls = 0;
    vi.mocked(createTerminal).mockResolvedValue(created);
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "running" });
    vi.mocked(closeTerminal).mockResolvedValue(workspace);
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
    expect(terminalLib.enableTerminalLigatures).toHaveBeenCalledTimes(1);
    expect(terminalLib.enableTerminalSelectionCopy).toHaveBeenCalledTimes(1);
    expect(terminalLib.attachTerminalRenderer).toHaveBeenCalledTimes(1);
    expect(writeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", new TextEncoder().encode("λ pasted"));
    expect(resizeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", 97, 31);
    wrapper.unmount();
  });

  it("fits a second time once the face has loaded, so the grid is not sized for the fallback's cell", async () => {
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    // The first fit measures the cell the fallback has, and the browser resolves the `local()`
    // face after it. Nothing else refits, so without the second one the grid stays short by the
    // columns and the row the two cells differ in — a strip down the right and along the bottom,
    // flush at the top-left corner, which is the one shape a padding bug cannot explain.
    expect(terminalLib.fitCalls).toBeGreaterThanOrEqual(2);
    expect(terminalMock.clearTextureAtlasCalls).toBe(1);
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

  it("keeps a completed process visible with its exit code", async () => {
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "exited", exitCode: 17 });
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    expect(wrapper.text()).not.toContain("Exited · code 17");
    expect(wrapper.emitted("statusChanged")).toEqual([[{ state: "exited", exitCode: 17 }]]);
    wrapper.unmount();
  });
});
