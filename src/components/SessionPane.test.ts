// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Session, WorkspaceState } from "../domain/workspace";
import { addSessionToLayout, createTerminalLayout } from "../domain/terminal-layout";
import { loadTerminalLayout, saveTerminalLayout } from "../lib/ipc";
import { useToasts } from "../presentation/toasts";
import SessionPane from "./SessionPane.vue";

const { toasts, dismiss } = useToasts();

const terminalMock = vi.hoisted(() => ({
  mounts: 0,
  autoCreate: false,
  closeRequests: 0,
  createdCount: 0,
  closedIds: [] as string[],
}));

vi.mock("../lib/ipc", () => ({
  loadTerminalLayout: vi.fn(),
  saveTerminalLayout: vi.fn(),
}));

vi.mock("./TerminalSession.vue", async () => {
  const { defineComponent: component, h: createElement, onMounted } = await import("vue");
  return {
    default: component({
      name: "TerminalSession",
      props: {
        checkoutId: { type: String, required: true },
        active: { type: Boolean, default: false },
        visible: { type: Boolean, default: true },
        sessionType: { type: String, default: "shell" },
        launchTarget: { type: Object, default: undefined },
        launchPrompt: { type: String, default: undefined },
      },
      emits: ["created", "closed", "statusChanged", "failed"],
      setup(props, { emit, expose }) {
        let sessionId: string | null = null;
        expose({
          requestClose: async () => {
            terminalMock.closeRequests++;
            if (sessionId) terminalMock.closedIds.push(sessionId);
            emit("closed", {
              repos: [],
              activeCheckoutId: props.checkoutId,
              activeSessionId: null,
            });
            return true;
          },
        });
        onMounted(() => {
          terminalMock.mounts++;
          if (!terminalMock.autoCreate) return;
          sessionId = terminalMock.createdCount++ === 0 ? "session:live" : `session:live-${terminalMock.createdCount}`;
          const session: Session = {
            id: sessionId,
            type: "shell",
            checkoutId: props.checkoutId,
            name: "zsh",
            createdAt: "now",
            status: "active",
          };
          const workspace: WorkspaceState = {
            repos: [],
            activeCheckoutId: props.checkoutId,
            activeSessionId: session.id,
          };
          emit("created", { session, workspace });
        });
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
  terminalMock.mounts = 0;
  terminalMock.autoCreate = false;
  terminalMock.closeRequests = 0;
  terminalMock.createdCount = 0;
  terminalMock.closedIds = [];
  vi.mocked(loadTerminalLayout).mockResolvedValue(null);
  vi.mocked(saveTerminalLayout).mockResolvedValue(undefined);
  for (const toast of [...toasts.value]) dismiss(toast.id);
});

/** The empty state's own action is how a terminal starts, now that picking a workdir no
 *  longer opens one. */
async function openTerminal(wrapper: ReturnType<typeof mount>) {
  const button = wrapper.findAll("button").find((candidate) => candidate.text() === "New terminal");
  expect(button, "the empty state offers New terminal").toBeDefined();
  button!.element.dispatchEvent(new Event("click", { bubbles: true }));
  await flushPromises();
}

describe("SessionPane terminal UI", () => {
  it("does not start a terminal on its own when a workdir is selected", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true },
    });
    await flushPromises();
    expect(loadTerminalLayout).not.toHaveBeenCalled();

    await wrapper.setProps({ isOpening: false });
    await flushPromises();

    // Selecting a workdir shows whatever it already has; it never spawns a terminal by
    // itself. A checkout with nothing open belongs on its empty state.
    expect(terminalMock.mounts).toBe(0);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(0);
    expect(wrapper.text()).toContain("No terminal session.");

    // The shell it used to open by surprise is still one click away, from that state.
    await openTerminal(wrapper);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    expect(wrapper.findComponent({ name: "TerminalSession" }).props("sessionType")).toBe("shell");
    // And it is still a bare shell, with no type or split controls.
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(0);
    expect(wrapper.findAll("select")).toHaveLength(0);
    expect(wrapper.text()).not.toContain("Split");
    wrapper.unmount();
  });

  it("keeps the pending view identity stable after IPC returns its persistent session ID", async () => {
    terminalMock.autoCreate = true;
    const wrapper = mount(SessionPane, {
      props: {
        checkout,
        activeSessionId: null,
        isOpening: true,
        shellRequest: null,
        registeredSessionIds: [],
      },
    });

    await wrapper.setProps({
      isOpening: false,
      shellRequest: { checkoutId: checkout.id, token: 1 },
    });
    await flushPromises();
    expect(wrapper.emitted("workspaceUpdated")).toHaveLength(1);

    const withCreatedSession = {
      ...checkout,
      sessions: [
        ...checkout.sessions,
        {
          id: "session:live",
          type: "shell" as const,
          checkoutId: checkout.id,
          name: "zsh",
          createdAt: "now",
          status: "active" as const,
        },
      ],
    };
    await wrapper.setProps({
      checkout: withCreatedSession,
      activeSessionId: "session:live",
      registeredSessionIds: ["session:live"],
    });
    await flushPromises();

    expect(terminalMock.mounts).toBe(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    expect(saveTerminalLayout).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("keeps the terminal mounted while document tabs hide and show it", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true, visible: true },
    });
    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);
    const terminal = wrapper.findComponent({ name: "TerminalSession" });
    expect(terminal.exists()).toBe(true);
    expect(terminal.props("visible")).toBe(true);

    await wrapper.setProps({ visible: false });
    expect(wrapper.findComponent({ name: "TerminalSession" }).exists()).toBe(true);
    expect(wrapper.findComponent({ name: "TerminalSession" }).props("visible")).toBe(false);
    await wrapper.setProps({ visible: true });
    expect(wrapper.findComponent({ name: "TerminalSession" }).props("visible")).toBe(true);
    expect(terminalMock.mounts).toBe(1);
    wrapper.unmount();
  });

  it("does not create another shell when an existing session is selected", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true },
    });
    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);
    expect(terminalMock.mounts).toBe(1);

    await wrapper.setProps({ activeSessionId: "session:old" });
    await flushPromises();
    expect(terminalMock.mounts).toBe(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    wrapper.unmount();
  });

  it("creates one terminal for a shell request on a checkout that has none", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true, shellRequest: null },
    });

    await wrapper.setProps({
      isOpening: false,
      shellRequest: { checkoutId: checkout.id, token: 1 },
    });
    await flushPromises();

    expect(terminalMock.mounts).toBe(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    wrapper.unmount();
  });

  it("answers a shell request that arrived before the workdir it is for", async () => {
    // Picking a workdir for the first time reads its saved UI state, and the pane is handed no
    // checkout until that read finishes. The request is made in between, so watching the token
    // alone would drop it and the click would have to be made twice.
    const wrapper = mount(SessionPane, {
      props: { checkout: null, activeSessionId: null, isOpening: false, shellRequest: null },
    });

    await wrapper.setProps({ shellRequest: { checkoutId: checkout.id, token: 1 } });
    await flushPromises();
    expect(terminalMock.mounts).toBe(0);

    await wrapper.setProps({ checkout });
    await flushPromises();

    expect(terminalMock.mounts).toBe(1);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);

    // The pair is watched, not the token alone: coming back to the same workdir later replays
    // the same props, and that is not a second click.
    const other: Checkout = { ...checkout, id: "checkout:/work/other", path: "/work/other" };
    await wrapper.setProps({ checkout: other });
    await flushPromises();
    await wrapper.setProps({ checkout });
    await flushPromises();

    expect(terminalMock.mounts).toBe(1);
    wrapper.unmount();
  });

  it("creates additional shells on request and closes only the requested session", async () => {
    terminalMock.autoCreate = true;
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true, shellRequest: null },
    });

    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);
    await wrapper.setProps({ shellRequest: { checkoutId: checkout.id, token: 2 } });
    await flushPromises();

    expect(terminalMock.mounts).toBe(2);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(2);
    await wrapper.vm.requestClose("session:live-2");

    expect(terminalMock.closedIds).toEqual(["session:live-2"]);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    wrapper.unmount();
  });

  it("repeatedly creates only one terminal after close without a watcher restart loop", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true },
    });
    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);

    wrapper.findComponent({ name: "TerminalSession" }).vm.$emit("closed", {
      repos: [],
      activeCheckoutId: checkout.id,
      activeSessionId: null,
    });
    await flushPromises();

    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(0);
    expect(wrapper.findAll("button").some((button) => button.text() === "New terminal")).toBe(true);
    expect(terminalMock.mounts).toBe(1);

    const createButton = wrapper.findAll("button").find((button) => button.text() === "New terminal")!;
    createButton.element.dispatchEvent(new Event("click", { bubbles: true }));
    createButton.element.dispatchEvent(new Event("click", { bubbles: true }));
    await flushPromises();

    expect(terminalMock.mounts).toBe(2);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    wrapper.unmount();
  });

  it("normalizes a legacy split layout and does not mount historical PTYs", async () => {
    const otherSession: Session = {
      id: "session:old-split",
      type: "shell",
      checkoutId: checkout.id,
      name: "old split",
      createdAt: "then",
      status: "inactive",
    };
    const legacyLayout = addSessionToLayout(createTerminalLayout([checkout.sessions[0]]), otherSession, {
      targetSessionId: checkout.sessions[0].id,
      direction: "vertical",
      splitId: "split:old",
    });
    vi.mocked(loadTerminalLayout).mockResolvedValue(legacyLayout);
    const wrapper = mount(SessionPane, {
      props: {
        checkout: { ...checkout, sessions: [...checkout.sessions, otherSession] },
        activeSessionId: null,
        isOpening: true,
      },
    });

    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);

    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(1);
    expect(terminalMock.mounts).toBe(1);
    expect(saveTerminalLayout).toHaveBeenCalledWith(
      checkout.id,
      expect.objectContaining({
        tabs: [expect.objectContaining({ root: { kind: "session", sessionId: "session:old" } })],
      }),
    );
    wrapper.unmount();
  });

  it("announces a terminal that will not start, with no error bar under the panel", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true },
    });
    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);

    wrapper.findComponent({ name: "TerminalSession" }).vm.$emit("failed", "pty could not be spawned");
    await flushPromises();

    expect(toasts.value.map((toast) => toast.message)).toEqual(["pty could not be spawned"]);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.findAllComponents({ name: "TerminalSession" })).toHaveLength(0);
    wrapper.unmount();
  });

  it("heals a mismatched persisted layout without showing an internal validation error", async () => {
    vi.mocked(loadTerminalLayout).mockResolvedValue({
      activeTabId: "tab:stale",
      tabs: [
        { id: "tab:old", root: { kind: "session", sessionId: "session:old" } },
        { id: "tab:duplicate", root: { kind: "session", sessionId: "session:old" } },
        { id: "tab:stale", root: { kind: "session", sessionId: "session:stale" } },
      ],
      sessionOrder: ["session:old", "session:old", "session:stale"],
    });
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: true },
    });

    await wrapper.setProps({ isOpening: false });
    await flushPromises();
    await openTerminal(wrapper);

    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(saveTerminalLayout).toHaveBeenCalledWith(
      checkout.id,
      expect.objectContaining({ sessionOrder: ["session:old"] }),
    );
    wrapper.unmount();
  });
});
