// @vitest-environment happy-dom
// Test doubles intentionally colocate small component shells and omit production prop defaults.
/* eslint-disable vue/one-component-per-file, vue/require-default-prop */
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, onMounted } from "vue";
import { DEFAULT_APP_LAYOUT, DEFAULT_CHECKOUT_UI_STATE } from "./domain/ui-state";
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
  onCloseRequested: null as ((event: { preventDefault(): void }) => Promise<void>) | null,
  currentWindow: null as {
    onCloseRequested: (handler: (event: { preventDefault(): void }) => Promise<void>) => Promise<() => void>;
    close: () => Promise<void>;
  } | null,
  onProgrammaticPanelResize: null as ((panelId: string, size: number) => void) | null,
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
      return () => h("div", { id: props.id }, slots.default?.());
    },
  });
  const SplitterResizeHandle = defineComponent({
    name: "SplitterResizeHandle",
    emits: ["dragging"],
    setup(_, { attrs }) {
      return () => h("div", { ...attrs, tabindex: 0 });
    },
  });
  return { SplitterGroup, SplitterPanel, SplitterResizeHandle };
});

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => mocks.currentWindow }));
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
  selectCheckout: vi.fn(),
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
        selectCheckout: async (checkoutId: string | null) => setActiveCheckout(checkoutId),
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
    status: null,
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

import { SplitterGroup, SplitterPanel, SplitterResizeHandle } from "reka-ui";
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
    return () => h("div", { "data-testid": "session-pane" });
  },
});

const InspectorPaneStub = defineComponent({
  name: "InspectorPane",
  props: { checkout: Object },
  emits: ["openFile", "updateUiState"],
  setup(props, { emit }) {
    return () =>
      h("div", [
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

function checkout(id: string, sessions: Session[] = []): Checkout {
  return {
    id,
    repoId: "repo:shared",
    path: `/${id}`,
    canonicalPath: `/${id}`,
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

async function mountApp(workspace: WorkspaceState, layout = { ...DEFAULT_APP_LAYOUT }) {
  mocks.initialWorkspace = workspace;
  mocks.loadAppLayout.mockResolvedValue(layout);
  const wrapper = mount(App, {
    global: {
      stubs: {
        Sidebar: SidebarStub,
        SessionPane: SessionPaneStub,
        InspectorPane: InspectorPaneStub,
        DocumentPane: DocumentPaneStub,
        WorktreeDialog: EmptyStub,
        GitStatusBar: EmptyStub,
        CommandPalette: EmptyStub,
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
    mocks.loadAppLayout.mockResolvedValue({ ...DEFAULT_APP_LAYOUT });
    mocks.loadCheckoutUiState.mockResolvedValue({ ...DEFAULT_CHECKOUT_UI_STATE });
    mocks.saveAppLayout.mockResolvedValue(undefined);
    mocks.saveCheckoutUiState.mockResolvedValue(undefined);
    mocks.getEditorAvailability.mockResolvedValue({ zed: false, neovim: false });
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

  it("does not persist synchronized splitter or viewport layouts over preferred widths", async () => {
    const preferred = { ...DEFAULT_APP_LAYOUT, sidebarWidth: 345, inspectorWidth: 450 };
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), preferred);
    const group = wrapper.getComponent(SplitterGroup);
    group.vm.$emit("layout", [220, 680, 260]);
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 950 });
    window.dispatchEvent(new Event("resize"));
    group.vm.$emit("layout", [0, 800, 0]);
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveAppLayout).not.toHaveBeenCalled();

    await wrapper.get('button[aria-label="Hide inspector"]').trigger("click");
    await flushPromises();
    await wrapper.get('button[aria-label="Show inspector"]').trigger("click");
    await wrapper.get("button.focus-button").trigger("click");
    await wrapper.get("button.focus-button").trigger("click");
    await flushPromises();
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarWidth: 345,
        inspectorWidth: 450,
        inspectorVisible: true,
      }),
    );
    expect(mocks.sessionPaneMounts).toBe(1);
    wrapper.unmount();
  });

  it("keeps both preferred widths while restoring a layout with both panels hidden", async () => {
    const preferred = {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 350,
      inspectorWidth: 500,
      sidebarVisible: false,
      inspectorVisible: false,
    };
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), preferred);
    wrapper.getComponent(SplitterGroup).vm.$emit("layout", [0, 900, 0]);
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 960 });
    window.dispatchEvent(new Event("resize"));
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveAppLayout).not.toHaveBeenCalled();

    await wrapper.get('button[aria-label="Show navigation"]').trigger("click");
    await wrapper.get('button[aria-label="Show inspector"]').trigger("click");
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarWidth: 350,
        inspectorWidth: 500,
        sidebarVisible: true,
        inspectorVisible: true,
      }),
    );
    wrapper.unmount();
  });

  it("persists user drag and collapse intent in pixel units and can reopen the panel once", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 345,
      inspectorWidth: 420,
    });
    const handles = wrapper.findAllComponents(SplitterResizeHandle);
    const panels = wrapper.findAllComponents(SplitterPanel);
    const group = wrapper.getComponent(SplitterGroup);
    handles[0]!.vm.$emit("dragging", true);
    group.vm.$emit("layout", [0, 600, 350]);
    panels[0]!.vm.$emit("collapse");
    handles[0]!.vm.$emit("dragging", false);
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarVisible: false,
        sidebarWidth: 345,
        inspectorWidth: 350,
      }),
    );

    await wrapper.get('button[aria-label="Show navigation"]').trigger("click");
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarVisible: true,
        sidebarWidth: 345,
      }),
    );
    wrapper.unmount();
  });

  it("ignores layout events caused by panel synchronization during an active drag", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 345,
      inspectorWidth: 420,
    });
    const group = wrapper.getComponent(SplitterGroup);
    const handles = wrapper.findAllComponents(SplitterResizeHandle);
    const panels = wrapper.findAllComponents(SplitterPanel);
    mocks.onProgrammaticPanelResize = () => group.vm.$emit("layout", [210, 830, 360]);

    handles[0]!.vm.$emit("dragging", true);
    panels[0]!.vm.$emit("collapse");
    await flushPromises();
    handles[0]!.vm.$emit("dragging", false);
    await flushPromises();

    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({ sidebarVisible: false, sidebarWidth: 345, inspectorWidth: 420 }),
    );
    wrapper.unmount();
  });

  it("lets double-click reset widths persist even though its resize callback is programmatic", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 345,
      inspectorWidth: 450,
    });
    const handle = wrapper.findAllComponents(SplitterResizeHandle)[0]!;
    await handle.trigger("dblclick");
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarWidth: DEFAULT_APP_LAYOUT.sidebarWidth,
        inspectorWidth: 450,
      }),
    );
    wrapper.unmount();
  });

  it("provides a global layout reset in the toolbar disclosure", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 345,
      inspectorWidth: 450,
    });
    const resizeCalls = vi.fn();
    mocks.onProgrammaticPanelResize = (panelId, size) => resizeCalls(panelId, size);
    await wrapper.get(".toolbar-settings summary").trigger("click");
    await wrapper.get(".toolbar-menu button:last-child").trigger("click");
    await flushPromises();
    expect(resizeCalls).toHaveBeenCalledWith("navigation-panel", DEFAULT_APP_LAYOUT.sidebarWidth);
    expect(resizeCalls).toHaveBeenCalledWith("inspector-panel", DEFAULT_APP_LAYOUT.inspectorWidth);
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarWidth: DEFAULT_APP_LAYOUT.sidebarWidth,
        inspectorWidth: DEFAULT_APP_LAYOUT.inspectorWidth,
        sidebarVisible: DEFAULT_APP_LAYOUT.sidebarVisible,
        inspectorVisible: DEFAULT_APP_LAYOUT.inspectorVisible,
      }),
    );
    wrapper.unmount();
  });

  it("persists keyboard splitter resizing using Reka's pixel layout values", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    const handle = wrapper.findAllComponents(SplitterResizeHandle)[0]!;
    await handle.trigger("keydown", { key: "ArrowRight" });
    wrapper.getComponent(SplitterGroup).vm.$emit("layout", [310, 750, 340]);
    await handle.trigger("keyup", { key: "ArrowRight" });
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenLastCalledWith(
      expect.objectContaining({
        sidebarWidth: 310,
        inspectorWidth: 340,
      }),
    );
    wrapper.unmount();
  });

  it("associates main tabs with panels, supports arrow keys, and keeps the terminal mounted", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    await wrapper.get('[data-testid="open-file"]').trigger("click");
    await flushPromises();
    const terminalTab = wrapper.get("#main-tab-terminal");
    const documentTab = wrapper.get("#main-tab-document");
    expect(terminalTab.attributes("aria-controls")).toBe("main-view-terminal");
    expect(documentTab.attributes("aria-controls")).toBe("main-view-document");
    expect(wrapper.get("#main-view-terminal").attributes("aria-labelledby")).toBe("main-tab-terminal");
    expect(wrapper.get("#main-view-document").attributes("aria-labelledby")).toBe("main-tab-document");

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

    await terminalTab.trigger("keydown", { key: "ArrowRight" });
    expect(documentTab.attributes("aria-selected")).toBe("true");
    await documentTab.trigger("keydown", { key: "ArrowLeft" });
    expect(terminalTab.attributes("aria-selected")).toBe("true");
    await wrapper.get('button[aria-label="Hide navigation"]').trigger("click");
    await wrapper.get("button.focus-button").trigger("click");
    expect(mocks.sessionPaneMounts).toBe(1);
    wrapper.unmount();
  });

  it("merges explicit terminal activation over deferred restore and ignores an older checkout load", async () => {
    const session: Session = {
      id: "session:two",
      type: "shell",
      checkoutId: "checkout:two",
      name: "Terminal",
      createdAt: "now",
      status: "active",
    };
    let resolveOne!: (state: unknown) => void;
    let resolveTwo!: (state: unknown) => void;
    mocks.loadCheckoutUiState.mockImplementation(
      (id: string) =>
        new Promise((resolve) => {
          if (id === "checkout:one") resolveOne = resolve;
          else resolveTwo = resolve;
        }),
    );
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two", [session])));
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
    expect(wrapper.get("#main-tab-terminal").attributes("aria-selected")).toBe("true");
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
    await wrapper.get('button[aria-label="Hide navigation"]').trigger("click");
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

  it("routes Command-Q through the same persistence flush", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    let resolveWrite!: () => void;
    mocks.saveAppLayout.mockImplementation(() => new Promise<void>((resolve) => (resolveWrite = resolve)));
    await wrapper.get('button[aria-label="Hide navigation"]').trigger("click");
    await flushPromises();
    const event = new KeyboardEvent("keydown", { key: "q", metaKey: true, cancelable: true });
    window.dispatchEvent(event);
    await flushPromises();
    expect(event.defaultPrevented).toBe(true);
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    resolveWrite();
    await flushPromises();
    expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
    wrapper.unmount();
  });
});
