// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";
import { addSessionToLayout, createTerminalLayout } from "../domain/terminal-layout";
import { loadTerminalLayout, saveTerminalLayout } from "../lib/ipc";
import SessionPane from "./SessionPane.vue";

vi.mock("../lib/ipc", () => ({
  loadTerminalLayout: vi.fn(),
  saveTerminalLayout: vi.fn(),
}));

vi.mock("./TerminalSession.vue", async () => {
  const { defineComponent: component, h: createElement } = await import("vue");
  return {
    default: component({
      name: "TerminalSession",
      props: {
        checkoutId: { type: String, required: true },
        active: { type: Boolean, default: false },
        sessionType: { type: String, default: "shell" },
        launchTarget: { type: Object, default: undefined },
        command: { type: String, default: null },
      },
      emits: ["created", "closed", "statusChanged", "failed"],
      setup(props) {
        return () => createElement("div", { "data-test": "terminal-session", "data-active": props.active });
      },
    }),
  };
});

const checkout: Checkout = {
  id: "checkout:/work/repo",
  repoId: "repo:/work/repo",
  path: "/work/repo",
  canonicalPath: "/work/repo",
  isPrimary: true,
  changedFiles: 0,
  isMissing: false,
  sessions: [
    {
      id: "session:old",
      type: "shell",
      checkoutId: "checkout:/work/repo",
      name: "zsh",
      createdAt: "now",
      status: "inactive",
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(loadTerminalLayout).mockResolvedValue(null);
  vi.mocked(saveTerminalLayout).mockResolvedValue(undefined);
});

describe("SessionPane terminal UI", () => {
  it("selects historical sessions and makes it clear their process was not restored", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false },
    });
    await flushPromises();

    await wrapper.get('[role="tab"]').trigger("click");
    await flushPromises();

    expect(wrapper.emitted("selectSession")).toEqual([["session:old"]]);
    await wrapper.setProps({ activeSessionId: "session:old" });
    expect(wrapper.text()).toContain("This session is inactive");
    expect(wrapper.text()).toContain("Its PTY was not restored");
  });

  it("starts a separate renderer view for a new shell and refuses a missing checkout", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false },
    });

    expect(wrapper.text()).toContain("New terminal");
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(wrapper.findAll('[data-test="terminal-session"]')).toHaveLength(1);
    expect(wrapper.get('[data-test="terminal-session"]').attributes("data-active")).toBe("true");

    await wrapper.setProps({ checkout: { ...checkout, isMissing: true } });
    expect(wrapper.get("header button").attributes("disabled")).toBeDefined();
  });

  it("routes an exact Neovim file location into a Marvis terminal session", async () => {
    const launchRequest = {
      checkoutId: checkout.id,
      filePath: "src/file; name.rs",
      line: 23,
      column: 8,
      token: 1,
    };
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false, nvimRequest: null },
    });
    await wrapper.setProps({ nvimRequest: launchRequest });
    await flushPromises();

    const terminal = wrapper.findComponent({ name: "TerminalSession" });
    expect(terminal.props("sessionType")).toBe("nvim");
    expect(terminal.props("launchTarget")).toEqual({
      filePath: "src/file; name.rs",
      line: 23,
      column: 8,
    });
    wrapper.unmount();
  });

  it("unmounts terminal views when their checkout session is removed", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false, registeredSessionIds: [] },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    const sessionView = wrapper.findComponent({ name: "TerminalSession" });
    sessionView.vm.$emit("created", {
      session: {
        id: "session:live",
        type: "shell",
        checkoutId: checkout.id,
        name: "zsh",
        createdAt: "now",
        status: "active",
      },
      workspace: { repos: [], activeCheckoutId: checkout.id, activeSessionId: "session:live" },
    });
    await flushPromises();
    expect(wrapper.findAll('[data-test="terminal-session"]')).toHaveLength(1);

    await wrapper.setProps({ registeredSessionIds: ["session:live"] });
    await wrapper.setProps({ registeredSessionIds: [] });

    expect(wrapper.findAll('[data-test="terminal-session"]')).toHaveLength(0);
  });

  it("creates nested horizontal and vertical splits as independent terminal sessions", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: "session:old", isOpening: false, allCheckouts: [checkout] },
    });

    await wrapper.get('[aria-label="Split horizontally (⌘D)"]').trigger("click");
    await flushPromises();
    let terminals = wrapper.findAllComponents({ name: "TerminalSession" });
    expect(terminals).toHaveLength(1);
    expect(terminals[0].props("checkoutId")).toBe(checkout.id);
    expect(terminals[0].props("sessionType")).toBe("shell");
    terminals[0].vm.$emit("created", {
      session: {
        id: "session:two",
        type: "shell",
        checkoutId: checkout.id,
        name: "zsh",
        createdAt: "now",
        status: "active",
      },
      workspace: { repos: [], activeCheckoutId: checkout.id, activeSessionId: "session:two" },
    });
    await flushPromises();

    const checkoutWithTwo = {
      ...checkout,
      sessions: [
        ...checkout.sessions,
        {
          id: "session:two",
          type: "shell" as const,
          checkoutId: checkout.id,
          name: "zsh",
          createdAt: "now",
          status: "active" as const,
        },
      ],
    };
    await wrapper.setProps({
      checkout: checkoutWithTwo,
      allCheckouts: [checkoutWithTwo],
      activeSessionId: "session:two",
    });
    await wrapper.get('[aria-label="Split vertically (⌘⇧D)"]').trigger("click");
    await flushPromises();
    terminals = wrapper.findAllComponents({ name: "TerminalSession" });
    expect(terminals).toHaveLength(2);
    terminals.at(-1)!.vm.$emit("created", {
      session: {
        id: "session:three",
        type: "shell",
        checkoutId: checkout.id,
        name: "zsh",
        createdAt: "now",
        status: "active",
      },
      workspace: { repos: [], activeCheckoutId: checkout.id, activeSessionId: "session:three" },
    });
    await flushPromises();

    const splitNodes = wrapper.findAll("[data-layout-split]");
    expect(splitNodes.map((node) => node.attributes("data-layout-split"))).toEqual(["horizontal", "vertical"]);
    expect(wrapper.findAll('[data-layout-session="session:old"]')).toHaveLength(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(2);
    expect(saveTerminalLayout).toHaveBeenCalledWith(
      checkout.id,
      expect.objectContaining({ sessionOrder: ["session:old", "session:two", "session:three"] }),
    );
    wrapper.unmount();
  });

  it("supports the keyboard shortcut for splitting the active session", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: "session:old", isOpening: false, allCheckouts: [checkout] },
    });
    const shortcut = new KeyboardEvent("keydown", { key: "d", metaKey: true, cancelable: true });
    window.dispatchEvent(shortcut);
    await flushPromises();

    expect(shortcut.defaultPrevented).toBe(true);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    wrapper.unmount();
  });

  it("does not offer generic server or custom executable launches from the WebView", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false },
    });

    expect(
      wrapper
        .get('[aria-label="New terminal session type"]')
        .findAll("option")
        .map((option) => option.attributes("value")),
    ).toEqual(["shell", "nvim"]);
    expect(wrapper.text()).toContain("Server/custom deferred; use Shell for commands.");
    wrapper.unmount();
  });

  it("restores checkout-specific split trees as inactive history without mounting PTYs", async () => {
    const first = {
      ...checkout,
      sessions: [
        checkout.sessions[0],
        {
          id: "session:first-split",
          type: "shell" as const,
          checkoutId: checkout.id,
          name: "old split",
          createdAt: "then",
          status: "inactive" as const,
        },
      ],
    };
    const second: Checkout = {
      ...checkout,
      id: "checkout:/work/other",
      repoId: "repo:/work/other",
      path: "/work/other",
      canonicalPath: "/work/other",
      sessions: [
        {
          id: "session:other",
          type: "nvim",
          checkoutId: "checkout:/work/other",
          name: "nvim",
          createdAt: "then",
          status: "inactive",
        },
      ],
    };
    const firstLayout = addSessionToLayout(createTerminalLayout([first.sessions[0]]), first.sessions[1], {
      targetSessionId: first.sessions[0].id,
      direction: "vertical",
      splitId: "split:first",
    });
    vi.mocked(loadTerminalLayout).mockImplementation(async (checkoutId) =>
      checkoutId === first.id ? firstLayout : createTerminalLayout(second.sessions),
    );

    const wrapper = mount(SessionPane, {
      props: { checkout: first, allCheckouts: [first, second], activeSessionId: null, isOpening: false },
    });
    await flushPromises();

    expect(wrapper.findAll('[data-layout-split="vertical"]')).toHaveLength(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(0);
    expect(wrapper.text()).toContain("Its PTY was not restored");
    expect(loadTerminalLayout).toHaveBeenCalledWith(first.id);
    expect(loadTerminalLayout).toHaveBeenCalledWith(second.id);
    expect(saveTerminalLayout).not.toHaveBeenCalled();

    await wrapper.setProps({ checkout: second, activeSessionId: "session:other" });
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(1);
    expect(wrapper.get('[role="tab"]').text()).toContain("nvim");
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(0);
    wrapper.unmount();
  });
});
