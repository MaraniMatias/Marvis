// @vitest-environment happy-dom
// Test doubles intentionally colocate small component shells and omit production prop defaults.
/* eslint-disable vue/one-component-per-file, vue/require-default-prop */
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, onMounted } from "vue";
import { DEFAULT_APP_LAYOUT, DEFAULT_CHECKOUT_UI_STATE } from "./domain/ui-state";
import type { AppLayoutState } from "./domain/ui-state";
import type { ReviewNote } from "./domain/review";
import type { Checkout, Repo, Session, WorkspaceState } from "./domain/workspace";

const mocks = vi.hoisted(() => ({
  initialWorkspace: null as WorkspaceState | null,
  workspaceRef: null as { value: WorkspaceState } | null,
  sessionPaneMounts: 0,
  saveAppLayout: vi.fn(),
  loadAppLayout: vi.fn(),
  loadCheckoutUiState: vi.fn(),
  saveCheckoutUiState: vi.fn(),
  getEditorAvailability: vi.fn(),
  toggleMaximize: vi.fn(),
  onCloseRequested: null as ((event: { preventDefault(): void }) => Promise<void>) | null,
  currentWindow: null as {
    onCloseRequested: (handler: (event: { preventDefault(): void }) => Promise<void>) => Promise<() => void>;
    close: () => Promise<void>;
  } | null,
  onProgrammaticPanelResize: null as ((panelId: string, size: number) => void) | null,
  gitStatus: null as { branch: string; defaultBranch: string } | null,
  reviewNotes: [] as ReviewNote[],
  agentSessions: [] as Array<{
    id: string;
    checkoutId: string;
    title: string;
    busy: boolean;
    blockedOnPermission: boolean;
    createdAt: number;
    updatedAt: number;
  }>,
  agentTargetId: null as string | null,
  createAgentSession: vi.fn(),
  sendAgentPrompt: vi.fn(),
  dispatchReviewRound: vi.fn(),
  reconcileRounds: vi.fn(),
  flushQueuedRounds: vi.fn(),
  ackFinishedTurn: vi.fn(),
  // Turn completions are observed, so the watcher needs a real reactive source.
  turns: null as { value: number } | null,
}));

vi.mock("reka-ui", async () => {
  const { defineComponent, h } = await import("vue");
  const SplitterGroup = defineComponent({
    name: "SplitterGroup",
    emits: ["layout"],
    setup(_, { attrs, slots }) {
      return () => h("div", attrs, slots.default?.());
    },
  });
  const SplitterPanel = defineComponent({
    name: "SplitterPanel",
    props: { id: String },
    emits: ["collapse", "expand"],
    setup(props, { emit, slots, expose }) {
      expose({
        collapse: () => emit("collapse"),
        expand: () => emit("expand"),
        resize: vi.fn((size: number) => mocks.onProgrammaticPanelResize?.(props.id ?? "", size)),
      });
      return () => h("div", { id: props.id, "data-collapsed": String(emit.length > 0) }, slots.default?.());
    },
  });
  const SplitterResizeHandle = defineComponent({
    name: "SplitterResizeHandle",
    emits: ["dragging"],
    setup(_, { attrs, slots }) {
      return () => h("div", { ...attrs, tabindex: 0 }, slots.default?.());
    },
  });
  // The titlebar's item crumb opens a popover; the stub always renders its content so the
  // sibling items can be reached without driving the open state.
  const passThrough = (name: string) =>
    defineComponent({
      name,
      setup(_, { attrs, slots }) {
        return () => h("div", attrs, slots.default?.());
      },
    });
  return {
    SplitterGroup,
    SplitterPanel,
    SplitterResizeHandle,
    PopoverRoot: passThrough("PopoverRoot"),
    PopoverTrigger: passThrough("PopoverTrigger"),
    PopoverContent: passThrough("PopoverContent"),
  };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ ...mocks.currentWindow, toggleMaximize: mocks.toggleMaximize }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(vi.fn()) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./lib/ipc", () => ({
  closeMissingCheckout: vi.fn(),
  getEditorAvailability: mocks.getEditorAvailability,
  loadAppLayout: mocks.loadAppLayout,
  loadCheckoutUiState: mocks.loadCheckoutUiState,
  locateMissingCheckout: vi.fn(),
  openInZed: vi.fn(),
  saveAppLayout: mocks.saveAppLayout,
  saveCheckoutUiState: mocks.saveCheckoutUiState,
  // The real command returns the refreshed workspace; App assigns it straight back,
  // so returning undefined here crashed the next render.
  selectCheckout: async (checkoutId: string | null) => {
    const workspace = mocks.workspaceRef;
    if (workspace) workspace.value = { ...workspace.value, activeCheckoutId: checkoutId };
    return workspace?.value;
  },
}));
vi.mock("./presentation/workspace", async () => {
  const { computed, ref } = await import("vue");
  return {
    useWorkspaceState: () => {
      const workspace = ref(mocks.initialWorkspace!);
      mocks.workspaceRef = workspace;
      const activeCheckout = computed(
        () =>
          workspace.value.repos
            .flatMap((repo) => repo.checkouts)
            .find((checkout) => checkout.id === workspace.value.activeCheckoutId) ?? null,
      );
      const isOpening = ref(false);
      const error = ref<string | null>(null);
      const setActiveCheckout = (checkoutId: string | null, sessionId: string | null = null) => {
        workspace.value = { ...workspace.value, activeCheckoutId: checkoutId, activeSessionId: sessionId };
      };
      return {
        workspace,
        activeCheckout,
        isOpening,
        error,
        chooseFolder: vi.fn(),
        selectCheckout: async (checkoutId: string | null) => {
          setActiveCheckout(checkoutId);
          return workspace.value;
        },
        selectSession: async (sessionId: string) => {
          const checkout = workspace.value.repos
            .flatMap((repo) => repo.checkouts)
            .find((item) => item.sessions.some((session) => session.id === sessionId));
          if (checkout) setActiveCheckout(checkout.id, sessionId);
        },
        updateWorkspace: (next: WorkspaceState) => (workspace.value = next),
        promptForDefaultBranchIfNeeded: vi.fn(),
      };
    },
  };
});
vi.mock("./presentation/active-git-snapshot", () => ({
  useActiveGitSnapshot: () => ({
    checkoutId: null,
    status: mocks.gitStatus ? { aheadCount: 1, files: [], ...mocks.gitStatus } : null,
    viewedPaths: [],
    loading: false,
    statusState: "idle",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: vi.fn(),
  }),
}));
vi.mock("./presentation/review-notes", () => ({
  useReviewNotes: () => ({
    checkoutId: "checkout:one",
    notes: mocks.reviewNotes,
    state: "ready",
    error: "",
    addNote: vi.fn(),
    updateNote: vi.fn(),
    deleteNote: vi.fn(),
    markSent: vi.fn(),
    verifyAnchors: vi.fn(),
    clearOutdated: vi.fn(),
    resolveNote: vi.fn(),
    dispatchRound: mocks.dispatchReviewRound,
    reconcileRounds: mocks.reconcileRounds,
    flushQueuedRounds: mocks.flushQueuedRounds,
    ackFinishedTurn: mocks.ackFinishedTurn,
  }),
}));
vi.mock("./presentation/agent-sessions", async () => {
  const { reactive } = await import("vue");
  const turns = reactive({ value: 0 });
  mocks.turns = turns;
  return {
    AGENT_EVENT: "marvis://agent-event",
    useAgentSessions: () => ({
      checkoutId: "checkout:one",
      sessions: mocks.agentSessions,
      targetId: mocks.agentTargetId,
      state: "ready",
      error: "",
      events: [],
      reload: vi.fn(),
      createSession: mocks.createAgentSession,
      sendReview: mocks.sendAgentPrompt,
      selectTarget: vi.fn(),
      stop: vi.fn(),
      // A getter, so the watcher that observes turn completions tracks this source.
      get turnsCompleted() {
        return turns.value;
      },
    }),
  };
});

import { SplitterGroup, SplitterResizeHandle } from "reka-ui";
import App from "./App.vue";

const SidebarStub = defineComponent({
  name: "SidebarStub",
  emits: ["selectCheckout", "selectSession"],
  setup(_, { emit }) {
    return () =>
      h("div", [
        h("button", { "data-testid": "select-checkout-two", onClick: () => emit("selectCheckout", "checkout:two") }),
        h("button", { "data-testid": "select-session-two", onClick: () => emit("selectSession", "session:two") }),
      ]);
  },
});

const SessionPaneStub = defineComponent({
  name: "SessionPane",
  setup(_, { expose }) {
    onMounted(() => (mocks.sessionPaneMounts += 1));
    expose({ focusActiveTerminal: vi.fn() });
    return () =>
      h("div", { "data-testid": "session-pane" }, [
        h("div", { class: "xterm" }, [h("textarea", { "data-testid": "terminal-input" })]),
      ]);
  },
});

const InspectorPaneStub = defineComponent({
  name: "InspectorPane",
  props: { checkout: Object },
  emits: ["openFile", "updateUiState", "sendReview"],
  setup(props, { emit }) {
    return () =>
      h("div", [
        h("button", {
          "data-testid": "send-review",
          onClick: () =>
            emit(
              "sendReview",
              (mocks.reviewNotes as ReviewNote[]).map((note) => note.id),
            ),
        }),
        h("button", {
          "data-testid": "open-file",
          disabled: !props.checkout,
          onClick: () =>
            props.checkout && emit("openFile", { checkoutId: (props.checkout as Checkout).id, path: "README.md" }),
        }),
        h("button", {
          "data-testid": "changes-scroll",
          disabled: !props.checkout,
          onClick: () =>
            emit("updateUiState", {
              inspectorTab: "changes",
              selectedFilePath: null,
              selectedChangePath: null,
              expandedDirectories: [],
              filesScrollTop: 0,
              changesScrollTop: 84,
            }),
        }),
      ]);
  },
});

const DocumentPaneStub = defineComponent({
  name: "DocumentPane",
  props: { document: Object },
  emits: ["diffPositionChanged", "readingPositionChanged"],
  setup(props, { emit }) {
    return () =>
      h("div", [
        h("span", { "data-testid": "document-pane" }, (props.document as { path: string }).path),
        h("button", { "data-testid": "diff-scroll", onClick: () => emit("diffPositionChanged", 132) }),
        h("button", {
          "data-testid": "document-scroll",
          onClick: () => emit("readingPositionChanged", { top: 240, left: 12 }),
        }),
      ]);
  },
});

const EmptyStub = defineComponent({ setup: () => () => h("div") });

function session(id: string, name: string, checkoutId = "checkout:one"): Session {
  return { id, type: "shell", checkoutId, name, createdAt: "now", status: "active" };
}

function checkout(id: string, sessions: Session[] = []): Checkout {
  return {
    id,
    repoId: "repo:shared",
    path: `/${id}`,
    canonicalPath: `/${id}`,
    branch: "main",
    isPrimary: id === "checkout:one",
    changedFiles: 0,
    isMissing: false,
    sessions,
  };
}

function workspaceWith(...checkouts: Checkout[]): WorkspaceState {
  const repo: Repo = {
    id: "repo:shared",
    kind: "git",
    name: "shared",
    root: "/",
    defaultBranch: "main",
    checkouts,
    createdAt: "now",
    lastOpenedAt: "now",
  };
  return { repos: [repo], activeCheckoutId: checkouts[0]?.id ?? null, activeSessionId: null };
}

async function mountApp(
  workspace: WorkspaceState,
  layout: AppLayoutState = { ...DEFAULT_APP_LAYOUT },
  options: { attachTo?: HTMLElement } = {},
) {
  mocks.initialWorkspace = workspace;
  mocks.loadAppLayout.mockResolvedValue(layout);
  const wrapper = mount(App, {
    attachTo: options.attachTo,
    global: {
      stubs: {
        Sidebar: SidebarStub,
        SessionPane: SessionPaneStub,
        InspectorPane: InspectorPaneStub,
        DocumentPane: DocumentPaneStub,
        WorktreeDialog: EmptyStub,
      },
    },
  });
  await flushPromises();
  await vi.advanceTimersByTimeAsync(300);
  await flushPromises();
  mocks.saveAppLayout.mockClear();
  mocks.saveCheckoutUiState.mockClear();
  return wrapper;
}

describe("App UI integration", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.resetAllMocks();
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
    mocks.initialWorkspace = null;
    mocks.workspaceRef = null;
    mocks.sessionPaneMounts = 0;
    mocks.onCloseRequested = null;
    mocks.onProgrammaticPanelResize = null;
    mocks.gitStatus = null;
    mocks.reviewNotes = [];
    mocks.agentSessions = [];
    mocks.agentTargetId = null;
    mocks.createAgentSession.mockReset();
    mocks.sendAgentPrompt.mockReset();
    mocks.dispatchReviewRound.mockReset();
    mocks.reconcileRounds.mockReset();
    mocks.flushQueuedRounds.mockReset();
    mocks.ackFinishedTurn.mockReset();
    if (mocks.turns) mocks.turns.value = 0;
    mocks.loadAppLayout.mockResolvedValue({ ...DEFAULT_APP_LAYOUT });
    mocks.loadCheckoutUiState.mockResolvedValue({ ...DEFAULT_CHECKOUT_UI_STATE });
    mocks.saveAppLayout.mockResolvedValue(undefined);
    mocks.saveCheckoutUiState.mockResolvedValue(undefined);
    mocks.getEditorAvailability.mockResolvedValue({ zed: false, neovim: false });
    mocks.toggleMaximize.mockResolvedValue(undefined);
    mocks.currentWindow = {
      onCloseRequested: vi.fn(async (handler) => {
        mocks.onCloseRequested = handler;
        return vi.fn();
      }),
      close: vi.fn(async () => undefined),
    };
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function reviewNoteFixture(): ReviewNote {
    return {
      id: "note:1",
      checkoutId: "checkout:one",
      path: "src/foo.js",
      side: "new",
      lineStart: 10,
      lineEnd: null,
      content: "revisit this calculation",
      code: "const result = a + b;",
      codeHash: "0000000000000001",
      outdated: false,
      roundId: null,
      status: "draft",
      createdAt: "1",
      updatedAt: "1",
    };
  }

  function agentSessionFixture(id: string, updatedAt: number) {
    return {
      id,
      checkoutId: "checkout:one",
      title: "review",
      busy: false,
      idleAt: 1,
      blockedOnPermission: false,
      createdAt: 1,
      updatedAt,
    };
  }

  describe("titlebar", () => {
    it("shows a real search field and no command palette hint", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      const search = wrapper.get('[data-testid="search-field"]');
      expect(search.element.tagName).toBe("INPUT");
      expect(search.attributes("aria-label")).toBe("Search files and commands");
      expect(search.attributes("placeholder")).toBe("Search...");
      expect(wrapper.find("kbd").exists()).toBe(false);
      expect(wrapper.findComponent({ name: "CommandPalette" }).exists()).toBe(false);
      wrapper.unmount();
    });

    it("takes focus and gives it back on Escape", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), undefined, {
        attachTo: document.body,
      });
      const search = wrapper.get('[data-testid="search-field"]').element as HTMLInputElement;

      search.focus();
      expect(document.activeElement).toBe(search);
      await search.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

      expect(document.activeElement).not.toBe(search);
      wrapper.unmount();
    });

    it("leaves an empty spacer for the window drag and zooms it on double click", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      const spacer = wrapper.get("[data-tauri-drag-region]");
      expect(spacer.text()).toBe("");
      expect(spacer.attributes("aria-hidden")).toBe("true");
      await spacer.trigger("dblclick");

      expect(mocks.toggleMaximize).toHaveBeenCalledOnce();
      wrapper.unmount();
    });

    it("derives the crumbs from the active checkout, branch and session", async () => {
      const wrapper = await mountApp(
        workspaceWith(
          checkout("checkout:one", [session("session:one", "Terminal 1"), session("session:two", "Neovim")]),
        ),
      );

      expect(wrapper.get('[data-testid="repo-crumb"]').text()).toBe("shared");
      expect(wrapper.get("nav").text()).toContain("main");
      // No session is selected yet, so the titlebar names the last one of the workdir.
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Neovim");
      wrapper.unmount();
    });

    it("opens the sibling terminals of the workdir from the last crumb", async () => {
      const wrapper = await mountApp(
        workspaceWith(
          checkout("checkout:one", [session("session:one", "Terminal 1"), session("session:two", "Neovim")]),
          checkout("checkout:two", [session("session:three", "Other", "checkout:two")]),
        ),
      );

      const siblings = wrapper.findAll(".surface-popover button");
      expect(siblings.map((button) => button.text())).toEqual(["Terminal 1", "Neovim"]);

      await siblings[0]!.trigger("click");
      await flushPromises();

      expect(mocks.workspaceRef?.value.activeSessionId).toBe("session:one");
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Terminal 1");
      wrapper.unmount();
    });

    it("keeps the settings gear without an action behind it", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      const settings = wrapper.get('[data-testid="settings-button"]');
      expect(settings.attributes("aria-label")).toBe("Settings");
      expect(settings.find("svg").exists()).toBe(true);
      wrapper.unmount();
    });

    it("renders no error banner", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      expect(wrapper.find('[role="alert"]').exists()).toBe(false);
      wrapper.unmount();
    });
  });

  describe("layout", () => {
    it("starts the panels at the marvis default widths within their clamps", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      const panels = wrapper.findAll("#navigation-panel, #inspector-panel");

      expect(panels[0]!.attributes()).toMatchObject({ "default-size": "240", "min-size": "240" });
      expect(panels[1]!.attributes()).toMatchObject({
        "default-size": "280",
        "min-size": "200",
        "max-size": "480",
      });
      expect(wrapper.get("#main-panel").attributes("min-size")).toBe("420");
      wrapper.unmount();
    });

    it("persists the widths a drag reports, in pixels", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [310, 750, 340]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 1,
        sidebarWidth: 310,
        inspectorWidth: 340,
      });
      wrapper.unmount();
    });

    it("clamps a drag that goes past the supported range", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [900, 400, 12]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 1,
        sidebarWidth: 500,
        inspectorWidth: 200,
      });
      wrapper.unmount();
    });

    it("keeps the inspector width when the narrow drawer reports a collapsed panel", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 1,
        sidebarWidth: 345,
        inspectorWidth: 450,
      });
      const group = wrapper.getComponent(SplitterGroup);
      const inspectorPanel = wrapper.findAllComponents({ name: "SplitterPanel" })[2]!;

      Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();
      expect(inspectorPanel.emitted("collapse")).toBeTruthy();

      group.vm.$emit("layout", [400, 500, 0]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 1,
        sidebarWidth: 400,
        inspectorWidth: 450,
      });
      wrapper.unmount();
    });

    it("lets double-click on a handle reset just that panel", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 1,
        sidebarWidth: 345,
        inspectorWidth: 450,
      });
      const resizeCalls = vi.fn();
      mocks.onProgrammaticPanelResize = resizeCalls;

      const handles = wrapper.findAllComponents(SplitterResizeHandle);
      expect(handles[0]!.find(".rounded-full").exists()).toBe(true);
      await handles[0]!.trigger("dblclick");
      await flushPromises();

      expect(resizeCalls).toHaveBeenCalledWith("navigation-panel", DEFAULT_APP_LAYOUT.sidebarWidth);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();
      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 1,
        sidebarWidth: DEFAULT_APP_LAYOUT.sidebarWidth,
        inspectorWidth: 450,
      });
      wrapper.unmount();
    });

    it("gives the inspector its full width back when space returns", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 1,
        sidebarWidth: 300,
        inspectorWidth: 300,
      });
      const inspectorPanel = wrapper.findAllComponents({ name: "SplitterPanel" })[2]!;
      const resizeCalls = vi.fn();
      mocks.onProgrammaticPanelResize = resizeCalls;

      Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();
      expect(inspectorPanel.emitted("collapse")).toBeTruthy();

      Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();

      expect(inspectorPanel.emitted("expand")).toBeTruthy();
      expect(resizeCalls).toHaveBeenCalledWith("inspector-panel", 300);
      wrapper.unmount();
    });
  });

  it("dispatches the review to the chosen agent session as one round", async () => {
    mocks.gitStatus = { branch: "feature", defaultBranch: "main" };
    mocks.reviewNotes = [reviewNoteFixture()];
    mocks.agentSessions = [agentSessionFixture("ses_one", 10)];
    mocks.agentTargetId = "ses_one";
    mocks.dispatchReviewRound.mockResolvedValue({
      id: "round:1",
      checkoutId: "checkout:one",
      sessionId: "ses_one",
      status: "dispatched",
      marker: "marvis-review:round:1",
      noteIds: ["note:1"],
      createdAt: "1",
      updatedAt: "2",
    });
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();

    expect(mocks.createAgentSession).not.toHaveBeenCalled();
    expect(mocks.dispatchReviewRound).toHaveBeenCalledTimes(1);
    const [target, ids, markdown] = mocks.dispatchReviewRound.mock.calls[0];
    expect(target).toBe("ses_one");
    expect(ids).toEqual(["note:1"]);
    expect(markdown).toContain("# Code Review ");
    expect(markdown).toContain("> `main..feature` — 1 file, 1 note");
    expect(markdown).toContain("```js{10}");
    expect(markdown).toContain("> revisit this calculation");
    wrapper.unmount();
  });

  it("sends the rounds held back for a busy agent as soon as a turn ends", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    expect(mocks.flushQueuedRounds).not.toHaveBeenCalled();

    mocks.turns!.value = 1;
    await flushPromises();

    // The flush runs before the round is acknowledged: a queued review goes out while the
    // agent is free, and the turn completion is what says it is.
    expect(mocks.flushQueuedRounds).toHaveBeenCalledTimes(1);
    expect(mocks.ackFinishedTurn).toHaveBeenCalledTimes(1);
    expect(mocks.flushQueuedRounds.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.ackFinishedTurn.mock.invocationCallOrder[0],
    );
    wrapper.unmount();
  });

  it("starts a session when the checkout has none, rather than dropping the review", async () => {
    mocks.gitStatus = { branch: "feature", defaultBranch: "main" };
    mocks.reviewNotes = [reviewNoteFixture()];
    mocks.agentSessions = [];
    mocks.agentTargetId = null;
    mocks.createAgentSession.mockResolvedValue(agentSessionFixture("ses_new", 10));
    mocks.dispatchReviewRound.mockResolvedValue({
      id: "round:1",
      checkoutId: "checkout:one",
      sessionId: "ses_new",
      status: "dispatched",
      marker: "marvis-review:round:1",
      noteIds: ["note:1"],
      createdAt: "1",
      updatedAt: "2",
    });
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();

    expect(mocks.createAgentSession).toHaveBeenCalledTimes(1);
    expect(mocks.dispatchReviewRound.mock.calls[0][0]).toBe("ses_new");
    wrapper.unmount();
  });

  it("switches the main view from the inspector and back from a crumb, without unmounting the terminal", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

    await wrapper.get('[data-testid="open-file"]').trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).toBe("none");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");
    // The document is not what the titlebar names: the crumb still points at the session.
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Terminal 1");

    await wrapper.get('[data-testid="diff-scroll"]').trigger("click");
    await wrapper.get('[data-testid="document-scroll"]').trigger("click");
    await wrapper.get('[data-testid="changes-scroll"]').trigger("click");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
      "checkout:one",
      expect.objectContaining({
        documentScrollTop: 240,
        documentScrollLeft: 12,
        diffScrollTop: 132,
        changesScrollTop: 84,
      }),
    );

    const siblings = wrapper.findAll(".surface-popover button");
    await siblings[0]!.trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).not.toBe("none");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).toBe("none");
    expect(mocks.sessionPaneMounts).toBe(1);
    wrapper.unmount();
  });

  it("merges explicit terminal activation over deferred restore and ignores an older checkout load", async () => {
    const other = session("session:two", "Other", "checkout:two");
    let resolveOne!: (state: unknown) => void;
    let resolveTwo!: (state: unknown) => void;
    mocks.loadCheckoutUiState.mockImplementation(
      (id: string) =>
        new Promise((resolve) => {
          if (id === "checkout:one") resolveOne = resolve;
          else resolveTwo = resolve;
        }),
    );
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two", [other])));
    await wrapper.get('[data-testid="select-checkout-two"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="select-session-two"]').trigger("click");
    await flushPromises();
    expect(mocks.saveCheckoutUiState).not.toHaveBeenCalled();

    resolveTwo({
      ...DEFAULT_CHECKOUT_UI_STATE,
      document: { checkoutId: "checkout:two", path: "two.md", source: "file", mode: "view" },
      mainView: "document",
    });
    await flushPromises();
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Other");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).toBe("none");
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("two.md");

    resolveOne({
      ...DEFAULT_CHECKOUT_UI_STATE,
      document: { checkoutId: "checkout:one", path: "one.md", source: "file", mode: "view" },
      mainView: "document",
    });
    await flushPromises();
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("two.md");
    expect(mocks.saveCheckoutUiState).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("waits for queued persistence before allowing a native close", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    let resolveWrite!: () => void;
    mocks.saveAppLayout.mockImplementation(() => new Promise<void>((resolve) => (resolveWrite = resolve)));
    wrapper.getComponent(SplitterGroup).vm.$emit("layout", [320, 700, 300]);
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenCalled();

    const preventDefault = vi.fn();
    const closeHandler = mocks.onCloseRequested!;
    const closing = closeHandler({ preventDefault });
    const duplicateClose = closeHandler({ preventDefault: vi.fn() });
    await flushPromises();
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    resolveWrite();
    await Promise.all([closing, duplicateClose]);
    expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
    wrapper.unmount();
  });
});
