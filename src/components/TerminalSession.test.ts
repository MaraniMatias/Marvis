// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import TerminalSession from "./TerminalSession.vue";

const terminalMock = vi.hoisted(() => ({
  channel: null as { onmessage: (buffer: ArrayBuffer) => void } | null,
  input: null as ((value: string) => void) | null,
  resizes: [] as Array<(size: { cols: number; rows: number }) => void>,
  output: [] as number[][],
  options: null as Record<string, unknown> | null,
  focusCalls: 0,
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class MockChannel {
    onmessage: (buffer: ArrayBuffer) => void = () => {};

    constructor() {
      terminalMock.channel = this;
    }
  },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class MockTerminal {
    cols = 80;
    rows = 24;
    options = {};
    constructor(options: Record<string, unknown>) {
      terminalMock.options = options;
    }
    loadAddon() {}
    open() {}
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
    dispose() {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class MockFitAddon {
    fit() {}
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
    terminalMock.options = null;
    terminalMock.focusCalls = 0;
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
    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, "shell", expect.anything(), undefined);
    expect(terminalMock.options).toMatchObject({
      fontFamily: '"FiraCode Nerd Font Mono", monospace',
      fontSize: 16,
      lineHeight: 1.2,
    });
    expect(writeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", new TextEncoder().encode("λ pasted"));
    expect(resizeTerminal).toHaveBeenCalledWith("checkout:repo", "session:new", 97, 31);
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

  it("launches Neovim through its typed session request", async () => {
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, sessionType: "nvim" },
    });
    await flushPromises();

    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, "nvim", expect.anything(), undefined);
    expect(wrapper.text()).not.toContain("Neovim running");
    wrapper.unmount();
  });

  it("passes the selected file and exact line to the in-app Neovim session", async () => {
    const launchTarget = { filePath: "src/main file.rs", line: 42, column: 7 };
    const wrapper = mount(TerminalSession, {
      props: { checkoutId: "checkout:repo", active: true, sessionType: "nvim", launchTarget },
    });
    await flushPromises();

    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, "nvim", expect.anything(), launchTarget);
    wrapper.unmount();
  });

  it("launches a terminal without any prompt, because reviews go to the agent bridge", async () => {
    const wrapper = mount(TerminalSession, {
      props: {
        checkoutId: "checkout:repo",
        active: true,
        sessionType: "shell",
      },
    });
    await flushPromises();

    expect(createTerminal).toHaveBeenCalledWith("checkout:repo", 80, 24, "shell", expect.anything(), undefined);
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
