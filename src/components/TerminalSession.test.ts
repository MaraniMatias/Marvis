// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import TerminalSession from "./TerminalSession.vue";

const terminalMock = vi.hoisted(() => ({
  channel: null as { onmessage: (buffer: ArrayBuffer) => void } | null,
  input: null as ((value: string) => void) | null,
  resize: null as ((size: { cols: number; rows: number }) => void) | null,
  output: [] as number[][],
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
    loadAddon() {}
    open() {}
    onData(callback: (value: string) => void) {
      terminalMock.input = callback;
    }
    onResize(callback: (size: { cols: number; rows: number }) => void) {
      terminalMock.resize = callback;
    }
    write(data: Uint8Array) {
      terminalMock.output.push(Array.from(data));
    }
    focus() {}
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
    terminalMock.resize = null;
    terminalMock.output = [];
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
    terminalMock.resize?.({ cols: 97, rows: 31 });
    await flushPromises();

    expect(terminalMock.output).toEqual([Array.from(output)]);
    expect(writeTerminal).toHaveBeenCalledWith("session:new", new TextEncoder().encode("λ pasted"));
    expect(resizeTerminal).toHaveBeenCalledWith("session:new", 97, 31);
    wrapper.unmount();
  });

  it("requires confirmation before closing a running shell", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(confirm).toHaveBeenCalledWith("This shell is still running. Close the session and stop its process?");
    expect(closeTerminal).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(closeTerminal).toHaveBeenCalledWith("session:new");
    expect(wrapper.emitted("closed")).toHaveLength(1);
    wrapper.unmount();
    confirm.mockRestore();
  });

  it("keeps a completed process visible with its exit code", async () => {
    vi.mocked(getTerminalStatus).mockResolvedValue({ state: "exited", exitCode: 17 });
    const wrapper = mount(TerminalSession, { props: { checkoutId: "checkout:repo", active: true } });
    await flushPromises();

    expect(wrapper.text()).toContain("Exited · code 17");
    expect(wrapper.emitted("statusChanged")).toEqual([[{ state: "exited", exitCode: 17 }]]);
    wrapper.unmount();
  });
});
