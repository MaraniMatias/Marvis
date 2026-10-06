// @vitest-environment happy-dom
// Test doubles intentionally colocate small component shells and omit production prop defaults.
/* eslint-disable vue/one-component-per-file, vue/require-default-prop */
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, inject, onMounted } from "vue";
import type { InjectionKey, Ref } from "vue";
import { DEFAULT_APP_LAYOUT, DEFAULT_CHECKOUT_UI_STATE } from "./domain/ui-state";
import type { AppLayoutState } from "./domain/ui-state";
import { ACKNOWLEDGEMENT, CREDITS, REPOSITORY } from "./domain/credits";
import { DEFAULT_SETTINGS, SETTINGS_SECTIONS, cloneSettings } from "./domain/settings";
import type { AppSettings } from "./domain/settings";
import type { ReviewNote } from "./domain/review";
import { REVIEW_SENDER } from "./presentation/review-notes";
import { useToasts } from "./presentation/toasts";
import { WORKDIR_ICONS } from "./presentation/workdir-icons";
import type { ReviewSender } from "./presentation/review-notes";
import type { Checkout, Repo, Session, WorkspaceState } from "./domain/workspace";
import FileIcon from "./components/FileIcon.vue";

const mocks = vi.hoisted(() => ({
  initialWorkspace: null as WorkspaceState | null,
  workspaceRef: null as { value: WorkspaceState } | null,
  launchCheckoutId: null as { value: string | null } | null,
  sessionPaneMounts: 0,
  saveAppLayout: vi.fn(),
  loadAppLayout: vi.fn(),
  saveSettings: vi.fn(),
  loadSettings: vi.fn(),
  loadCheckoutUiState: vi.fn(),
  saveCheckoutUiState: vi.fn(),
  prepareAppExit: vi.fn(),
  getTerminalStatus: vi.fn(),
  loadReviewTarget: vi.fn(),
  saveReviewTarget: vi.fn(),
  exportReviewMarkdown: vi.fn(),
  closeCheckout: vi.fn(),
  closeMissingCheckout: vi.fn(),
  archiveCheckout: vi.fn(),
  restoreArchivedWorktrees: vi.fn(),
  renameTerminal: vi.fn(),
  listRecentPaths: vi.fn(),
  openPath: vi.fn(),
  selectCheckout: vi.fn(),
  restoreWorkspace: vi.fn(),
  toggleMaximize: vi.fn(),
  minimize: vi.fn(),
  isDecorated: vi.fn(),
  isMaximized: vi.fn(),
  onWindowResized: null as (() => void) | null,
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
  dispatchReviewRound: vi.fn(),
  reconcileRounds: vi.fn(),
  flushQueuedRounds: vi.fn(),
  ackFinishedTurn: vi.fn(),
  // Turn completions are observed, so the watcher needs a real reactive source.
  turns: null as { value: number } | null,
}));

vi.mock("reka-ui", async () => {
  const { computed, defineComponent, h, inject, provide } = await import("vue");
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
  // The titlebar's crumbs open menus; the stub always renders their content so the rows can be
  // reached without driving the open state. A row stands in for a menu item: it takes the attrs
  // it is given and reports a selection, which is what a click on it does.
  const passThrough = (name: string) =>
    defineComponent({
      name,
      setup(_, { attrs, slots }) {
        return () => h("div", attrs, slots.default?.());
      },
    });
  const menuRow = defineComponent({
    name: "DropdownMenuItem",
    inheritAttrs: false,
    props: { disabled: Boolean },
    setup(_, { attrs, emit, slots }) {
      return () => h("button", { ...attrs, onClick: () => emit("select") }, slots.default?.());
    },
  });
  // The root takes the open name as a prop, the way reka's does, so "only one menu at a time"
  // can be asserted on what the shell decided rather than on what a click did.
  const menuRoot = defineComponent({
    name: "DropdownMenuRoot",
    props: { open: Boolean },
    emits: ["update:open"],
    setup(props, { slots }) {
      return () => h("div", { "data-open": String(props.open) }, slots.default?.());
    },
  });
  const menuFilter = defineComponent({
    name: "DropdownMenuFilter",
    props: { modelValue: String, placeholder: String },
    emits: ["update:modelValue"],
    setup(props, { emit }) {
      return () =>
        h("input", {
          "data-testid": "menu-search",
          placeholder: props.placeholder,
          value: props.modelValue,
          onInput: (event: Event) => emit("update:modelValue", (event.target as HTMLInputElement).value),
        });
    },
  });
  // The Settings dialog's pickers are the only selects in the shell, and they are stubbed as a
  // list that is always open: the trigger carries the testid and the current value, and a row
  // reports a choice, which is what clicking one does.
  // Typed on the symbol rather than at each `inject`, so the trigger and the rows cannot disagree
  // about the shape of what the root provides.
  const selected = Symbol("selected") as InjectionKey<{
    value: Readonly<Ref<string | undefined>>;
    choose: (value: string) => void;
  }>;
  const SelectRoot = defineComponent({
    name: "SelectRoot",
    props: { modelValue: String },
    emits: ["update:modelValue"],
    setup(props, { emit, slots }) {
      provide(selected, {
        value: computed(() => props.modelValue),
        choose: (value: string) => emit("update:modelValue", value),
      });
      return () => slots.default?.();
    },
  });
  const SelectTrigger = defineComponent({
    name: "SelectTrigger",
    inheritAttrs: false,
    setup(_, { attrs, slots }) {
      return () => h("button", attrs, slots.default?.());
    },
  });
  const SelectValue = defineComponent({
    name: "SelectValue",
    setup() {
      const current = inject(selected)!;
      return () => current.value.value;
    },
  });
  const SelectItem = defineComponent({
    name: "SelectItem",
    props: { value: String },
    setup(props, { slots }) {
      const current = inject(selected)!;
      return () =>
        h(
          "button",
          {
            type: "button",
            "data-value": props.value,
            onClick: () => current.choose(props.value ?? ""),
          },
          slots.default?.(),
        );
    },
  });
  const selectPassThrough = (name: string) =>
    defineComponent({
      name,
      inheritAttrs: false,
      setup(_, { attrs, slots }) {
        return () => h("div", attrs, slots.default?.());
      },
    });

  return {
    SplitterGroup,
    SplitterPanel,
    SplitterResizeHandle,
    SelectRoot,
    SelectTrigger,
    SelectValue,
    SelectContent: selectPassThrough("SelectContent"),
    SelectPortal: selectPassThrough("SelectPortal"),
    SelectViewport: selectPassThrough("SelectViewport"),
    SelectItem,
    SelectItemIndicator: selectPassThrough("SelectItemIndicator"),
    SelectItemText: selectPassThrough("SelectItemText"),
    DropdownMenuRoot: menuRoot,
    DropdownMenuTrigger: passThrough("DropdownMenuTrigger"),
    DropdownMenuContent: passThrough("DropdownMenuContent"),
    DropdownMenuPortal: passThrough("DropdownMenuPortal"),
    DropdownMenuSeparator: passThrough("DropdownMenuSeparator"),
    DropdownMenuFilter: menuFilter,
    DropdownMenuItem: menuRow,
    // A group and its label only wrap the rows: the stub always renders their content, which is
    // what lets a grouped menu be reached without driving the open state, the same as the rest.
    DropdownMenuGroup: passThrough("DropdownMenuGroup"),
    DropdownMenuLabel: passThrough("DropdownMenuLabel"),
  };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    ...mocks.currentWindow,
    toggleMaximize: mocks.toggleMaximize,
    minimize: mocks.minimize,
    isDecorated: mocks.isDecorated,
    isMaximized: mocks.isMaximized,
    onResized: vi.fn(async (handler: () => void) => {
      mocks.onWindowResized = handler;
      return vi.fn();
    }),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(vi.fn()) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./lib/ipc", () => ({
  archiveCheckout: mocks.archiveCheckout,
  closeCheckout: mocks.closeCheckout,
  closeMissingCheckout: mocks.closeMissingCheckout,
  restoreArchivedWorktrees: mocks.restoreArchivedWorktrees,
  listRecentPaths: mocks.listRecentPaths,
  exportReviewMarkdown: mocks.exportReviewMarkdown,
  loadReviewTarget: mocks.loadReviewTarget,
  loadAppLayout: mocks.loadAppLayout,
  loadCheckoutUiState: mocks.loadCheckoutUiState,
  renameTerminal: mocks.renameTerminal,
  saveAppLayout: mocks.saveAppLayout,
  loadSettings: mocks.loadSettings,
  saveSettings: mocks.saveSettings,
  saveCheckoutUiState: mocks.saveCheckoutUiState,
  prepareAppExit: mocks.prepareAppExit,
  getTerminalStatus: mocks.getTerminalStatus,
  saveReviewTarget: mocks.saveReviewTarget,
  // The real command returns the refreshed workspace; App assigns it straight back,
  // so returning undefined here crashed the next render. Only the shell and Neovim requests
  // reach it from the titlebar, so it also reports that a terminal was asked for.
  selectCheckout: mocks.selectCheckout,
  restoreWorkspace: mocks.restoreWorkspace,
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
      const launchCheckoutId = ref<string | null>(null);
      mocks.launchCheckoutId = launchCheckoutId;
      const error = ref<string | null>(null);
      const setActiveCheckout = (checkoutId: string | null, sessionId: string | null = null) => {
        workspace.value = { ...workspace.value, activeCheckoutId: checkoutId, activeSessionId: sessionId };
      };
      return {
        workspace,
        activeCheckout,
        launchCheckoutId,
        isOpening,
        error,
        chooseFolder: vi.fn(),
        openPath: mocks.openPath,
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
    loading: false,
    statusState: "idle",
    statusError: "",
    changesStatusError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
  }),
}));
vi.mock("./presentation/git-watchers", () => ({
  // The watchers are the backend's business; this suite is about what the app does with the
  // workspace, not about when a repository starts being observed.
  useGitWatchers: vi.fn(),
}));
vi.mock("./presentation/review-notes", () => ({
  // The diff reaches the send through this key, so the real one has to be here.
  REVIEW_SENDER: Symbol("marvis:review-sender"),
  useReviewNotes: () => ({
    checkoutId: "checkout:one",
    notes: mocks.reviewNotes,
    rounds: [],
    state: "ready",
    error: "",
    addNote: vi.fn(),
    updateNote: vi.fn(),
    deleteNote: vi.fn(),
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
      selectTarget: vi.fn(),
      stop: vi.fn(),
      // A getter, so the watcher that observes turn completions tracks this source.
      get turnsCompleted() {
        return turns.value;
      },
    }),
    // The sidebar rows read this, keyed by checkout. Empty here: these tests are about the rest
    // of the app, and a row with no agent behind it is drawn without a chip.
    useTerminalAgentRows: () => ({
      byCheckout: {},
      row: () => ({ agent: null, running: false }),
      reload: vi.fn(),
    }),
  };
});

import { SplitterGroup, SplitterResizeHandle } from "reka-ui";
import App from "./App.vue";

const SidebarStub = defineComponent({
  name: "SidebarStub",
  emits: [
    "selectCheckout",
    "selectSession",
    "closeWorkdir",
    "closeMissing",
    "removeWorktree",
    "restoreArchived",
    "renameSession",
    "newTerminal",
  ],
  setup(_, { emit }) {
    return () =>
      h("div", [
        h("button", { "data-testid": "select-checkout-two", onClick: () => emit("selectCheckout", "checkout:two") }),
        h("button", { "data-testid": "select-session-one", onClick: () => emit("selectSession", "session:one") }),
        h("button", { "data-testid": "select-session-two", onClick: () => emit("selectSession", "session:two") }),
        h("button", { "data-testid": "new-terminal-one", onClick: () => emit("newTerminal", "checkout:one") }),
        h("button", { "data-testid": "close-workdir-base", onClick: () => emit("closeWorkdir", "checkout:one") }),
        h("button", { "data-testid": "close-workdir-worktree", onClick: () => emit("closeWorkdir", "checkout:two") }),
        h("button", { "data-testid": "close-missing-base", onClick: () => emit("closeMissing", "checkout:one") }),
        h("button", { "data-testid": "close-missing-worktree", onClick: () => emit("closeMissing", "checkout:two") }),
        // The row's own way off a worktree: one cross, and it opens the dialog that holds both
        // answers rather than picking one for the user.
        h("button", {
          "data-testid": "remove-worktree-two",
          onClick: () => emit("removeWorktree", "checkout:two"),
        }),
        h("button", {
          "data-testid": "restore-archived-shared",
          onClick: () => emit("restoreArchived", "repo:shared"),
        }),
        h("button", {
          "data-testid": "rename-session-one",
          onClick: () => emit("renameSession", "session:one", "build logs"),
        }),
      ]);
  },
});

const SessionPaneStub = defineComponent({
  name: "SessionPane",
  emits: ["sessionStatusChanged"],
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
  emits: ["openFile", "openAllChanges", "updateUiState"],
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
          "data-testid": "open-nested-file",
          disabled: !props.checkout,
          onClick: () =>
            props.checkout && emit("openFile", { checkoutId: (props.checkout as Checkout).id, path: "src/lib/one.ts" }),
        }),
        h("button", {
          "data-testid": "open-all-changes",
          disabled: !props.checkout,
          onClick: () => props.checkout && emit("openAllChanges", { checkoutId: (props.checkout as Checkout).id }),
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
  props: { path: String },
  emits: ["readingPositionChanged"],
  setup(props, { emit }) {
    return () =>
      h("div", [
        h("span", { "data-testid": "document-pane" }, props.path ?? ""),
        h("button", {
          "data-testid": "document-scroll",
          onClick: () => emit("readingPositionChanged", { top: 240, left: 12 }),
        }),
      ]);
  },
});

// The diff is where the send lives, so the stub takes the shell's sender the same way the real
// component does and asks it to hand over the notes, which is the wiring under test here.
const FileDiffStub = defineComponent({
  name: "FileDiff",
  props: { path: String },
  emits: ["scrollPositionChanged"],
  setup(props, { emit }) {
    const sender = inject<ReviewSender | null>(REVIEW_SENDER, null);
    return () =>
      h("div", [
        h("span", { "data-testid": "file-diff" }, props.path ?? "all"),
        h("button", { "data-testid": "diff-scroll", onClick: () => emit("scrollPositionChanged", 132) }),
        h("button", {
          "data-testid": "send-review",
          onClick: () =>
            void sender?.send(
              (mocks.reviewNotes as ReviewNote[]).map((note) => note.id),
              false,
            ),
        }),
      ]);
  },
});

// The dialog itself is not what the titlebar is judged on; what matters there is which
// lifecycle it was asked for and against which checkout, so the stub reports both.
const WorktreeDialogStub = defineComponent({
  name: "WorktreeDialog",
  props: { open: Boolean, mode: String, checkout: Object },
  setup(props) {
    return () =>
      h("div", {
        "data-testid": "worktree-dialog",
        "data-open": String(props.open),
        "data-mode": props.mode,
        "data-checkout": (props.checkout as Checkout | null)?.id ?? "",
      });
  },
});

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
  options: { attachTo?: HTMLElement; settings?: AppSettings; settingsLoad?: Promise<AppSettings> } = {},
) {
  mocks.initialWorkspace = workspace;
  mocks.loadAppLayout.mockResolvedValue(layout);
  if (options.settingsLoad) mocks.loadSettings.mockReturnValue(options.settingsLoad);
  else mocks.loadSettings.mockResolvedValue(options.settings ?? cloneSettings(DEFAULT_SETTINGS));
  const wrapper = mount(App, {
    attachTo: options.attachTo,
    global: {
      stubs: {
        Sidebar: SidebarStub,
        SessionPane: SessionPaneStub,
        InspectorPane: InspectorPaneStub,
        DocumentPane: DocumentPaneStub,
        FileDiff: FileDiffStub,
        WorktreeDialog: WorktreeDialogStub,
      },
    },
  });
  await flushPromises();
  await vi.advanceTimersByTimeAsync(300);
  await flushPromises();
  mocks.saveAppLayout.mockClear();
  mocks.saveSettings.mockClear();
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
    mocks.dispatchReviewRound.mockReset();
    mocks.reconcileRounds.mockReset();
    mocks.flushQueuedRounds.mockReset();
    mocks.ackFinishedTurn.mockReset();
    if (mocks.turns) mocks.turns.value = 0;
    mocks.loadAppLayout.mockResolvedValue({ ...DEFAULT_APP_LAYOUT });
    mocks.loadSettings.mockResolvedValue(cloneSettings(DEFAULT_SETTINGS));
    mocks.listRecentPaths.mockResolvedValue([]);
    mocks.selectCheckout.mockImplementation(async (checkoutId: string | null) => {
      const workspace = mocks.workspaceRef;
      if (workspace) workspace.value = { ...workspace.value, activeCheckoutId: checkoutId };
      return workspace?.value;
    });
    mocks.loadCheckoutUiState.mockResolvedValue({ ...DEFAULT_CHECKOUT_UI_STATE });
    mocks.saveAppLayout.mockResolvedValue(undefined);
    mocks.saveSettings.mockResolvedValue(undefined);
    mocks.saveCheckoutUiState.mockResolvedValue(undefined);
    mocks.prepareAppExit.mockResolvedValue(undefined);
    mocks.getTerminalStatus.mockResolvedValue({ state: "running", foregroundProcess: false });
    mocks.loadReviewTarget.mockResolvedValue("markdown");
    mocks.saveReviewTarget.mockResolvedValue(undefined);
    mocks.exportReviewMarkdown.mockResolvedValue("2026-03-14-1532.md");
    mocks.toggleMaximize.mockResolvedValue(undefined);
    mocks.minimize.mockResolvedValue(undefined);
    // A window with a frame of its own is the case the app was written against, so it is what
    // every test starts from; the ones that care about the frameless window say so.
    mocks.isDecorated.mockResolvedValue(true);
    mocks.isMaximized.mockResolvedValue(false);
    mocks.onWindowResized = null;
    mocks.currentWindow = {
      onCloseRequested: vi.fn(async (handler) => {
        mocks.onCloseRequested = handler;
        return vi.fn();
      }),
      close: vi.fn(async () => undefined),
    };
  });

  afterEach(() => {
    // The toast stack is a module singleton on purpose (A.6), so a message one test raises is
    // still up for the next one. Each test starts with an empty stack.
    const { toasts } = useToasts();
    toasts.value = [];
    // The scale is written to the document rather than to the app, so it outlives the component
    // that set it and would be the next test's starting point.
    document.documentElement.style.removeProperty("zoom");
    document.documentElement.style.removeProperty("--marvis-ui-font-scale");
    delete document.documentElement.dataset.theme;
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
    it("leaves an empty spacer for the window to drag, and the double click to the window", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      const spacer = wrapper.get("[data-tauri-drag-region]");
      expect(spacer.text()).toBe("");
      expect(spacer.attributes("aria-hidden")).toBe("true");
      // The window reads the double click off the drag region itself. Answering it here as
      // well toggled twice, so the window grew and came straight back.
      await spacer.trigger("dblclick");

      expect(mocks.toggleMaximize).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("leaves the window's controls to the frame when the window has one", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      expect(wrapper.find('[data-testid="window-close"]').exists()).toBe(false);
      // The room macOS's traffic lights need is left alone.
      expect(wrapper.get("header").classes()).toContain("pl-[78px]");
      wrapper.unmount();
    });

    it("draws minimize, maximize and close on a window that has no frame of its own", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      // No frame means no traffic lights, so the space they took goes to the crumbs.
      expect(wrapper.get("header").classes()).toContain("pl-3");
      await wrapper.get('[data-testid="window-minimize"]').trigger("click");
      expect(mocks.minimize).toHaveBeenCalledOnce();
      await wrapper.get('[data-testid="window-maximize"]').trigger("click");
      expect(mocks.toggleMaximize).toHaveBeenCalledOnce();
      await flushPromises();

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();
      expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
      wrapper.unmount();
    });

    it("names the maximize control after what the window is, and follows the desktop", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      expect(wrapper.get('[data-testid="window-maximize"]').attributes("aria-label")).toBe("Maximize window");

      // The desktop maximizes the window on its own, on a double click over the drag region or
      // on its own shortcut, and the control has to offer to restore rather than to maximize.
      mocks.isMaximized.mockResolvedValue(true);
      mocks.onWindowResized!();
      await flushPromises();

      expect(wrapper.get('[data-testid="window-maximize"]').attributes("aria-label")).toBe("Restore window");
      wrapper.unmount();
    });

    it("waits for the queued persistence before closing from its own control", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      let resolveWrite!: () => void;
      mocks.saveAppLayout.mockImplementation(() => new Promise<void>((resolve) => (resolveWrite = resolve)));
      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [320, 700, 300]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();
      expect(mocks.currentWindow!.close).not.toHaveBeenCalled();

      resolveWrite();
      await flushPromises();
      expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
      wrapper.unmount();
    });

    it("ends the processes the app started before the window goes", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      const order: string[] = [];
      mocks.prepareAppExit.mockImplementation(async () => {
        order.push("sweep");
      });
      mocks.currentWindow!.close = vi.fn(async () => {
        order.push("close");
      });

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();

      // The sweep has to happen while the window is still up to ask for it: no exit event reaches
      // the app on this path, so nothing else ends the servers the app started.
      expect(order).toEqual(["sweep", "close"]);
      wrapper.unmount();
    });

    it("closes even when the sweep before it fails", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      mocks.prepareAppExit.mockRejectedValue(new Error("the sweep did not finish"));

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();

      // A sweep that failed is a server left running, not a window the user cannot close.
      expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
      wrapper.unmount();
    });

    it("closes anyway when a queued write never finishes", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      // A write that answers nothing is what a stuck bridge looks like from here, and a window
      // that waits on it forever is a window the user cannot close.
      mocks.saveAppLayout.mockImplementation(() => new Promise<void>(() => {}));
      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [320, 700, 300]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();
      expect(mocks.currentWindow!.close).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(5000);
      await flushPromises();
      expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
      wrapper.unmount();
    });

    it("asks again when the last close did not go through", async () => {
      mocks.isDecorated.mockResolvedValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      const refused = vi.fn(async () => {
        throw new Error("the window would not close");
      });
      mocks.currentWindow!.close = refused;

      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();
      expect(refused).toHaveBeenCalledOnce();

      // A close that failed cannot keep the window from being asked to close again.
      const accepted = vi.fn(async () => undefined);
      mocks.currentWindow!.close = accepted;
      await wrapper.get('[data-testid="window-close"]').trigger("click");
      await flushPromises();
      expect(accepted).toHaveBeenCalledOnce();
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

    it("wears the icon the sidebar gives the row each crumb names", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

      // The line is the sidebar read sideways, so a worktree and a terminal wear the two glyphs
      // their rows wear over there. The workdir wears nothing: it is the one name that is always
      // there, and a glyph on it would only repeat a word that never changes.
      expect(wrapper.get('[data-testid="repo-crumb"]').find(".crumb-icon").exists()).toBe(false);
      expect(wrapper.get('[data-testid="worktree-crumb"]').findComponent(WORKDIR_ICONS.worktree).exists()).toBe(true);
      expect(wrapper.get('[data-testid="item-crumb"]').findComponent(WORKDIR_ICONS.terminal).exists()).toBe(true);
      wrapper.unmount();
    });

    it("wears the file's own icon on the step that names it, and on the probe that weighs the path", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      await wrapper.get('[data-testid="open-nested-file"]').trigger("click");
      await flushPromises();

      // The one icon in the crumb, and it is the last thing before the file it names: an icon at
      // the head of the path would name the first directory rather than what is open.
      const crumb = wrapper.get('[data-testid="item-crumb"]');
      expect(crumb.findAll(".crumb-icon")).toHaveLength(1);
      // The glyph stands immediately before the name it belongs to, and after the separator.
      expect(crumb.element.querySelector(".crumb-icon-slot")?.nextSibling?.textContent).toBe("one.ts");
      expect(crumb.text()).toBe("src/lib/one.ts");
      // The probe is the crumb at the width it wants, glyph included: a path measured without it
      // would run past the line before it was told to give anything up.
      expect(wrapper.get('[data-testid="path-probe"]').findComponent(FileIcon).exists()).toBe(true);
      wrapper.unmount();
    });

    it("leaves the last crumb bare when what it names is not a file", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));
      await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
      await flushPromises();

      // The whole change set is not one file, so there is no file's icon to wear.
      const crumb = wrapper.get('[data-testid="item-crumb"]');
      expect(crumb.text()).toBe("All changes");
      expect(crumb.find("svg").exists()).toBe(false);
      wrapper.unmount();
    });

    it("opens every open workdir from the first crumb, and takes the one that is picked", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")]), {
          ...checkout("checkout:two"),
          branch: "feature",
        }),
      );

      const rows = wrapper.findAll('[data-testid^="menu-item-workdir:"]');
      // Both checkouts of the repo, named the way the sidebar names them.
      expect(rows.map((row) => row.text())).toEqual(["mainshared", "featureshared"]);

      await wrapper.get('[data-testid="menu-item-workdir:checkout:two"]').trigger("click");
      await flushPromises();

      expect(mocks.workspaceRef?.value.activeCheckoutId).toBe("checkout:two");
      wrapper.unmount();
    });

    it("lists a folder opened before under the workdirs, and opens it again", async () => {
      mocks.listRecentPaths.mockResolvedValue([
        { canonicalPath: "/Trabajo/skills", lastOpenedAt: "1" },
        // Already open in this window, so it belongs to This Window and not here.
        { canonicalPath: "/checkout:one", lastOpenedAt: "2" },
      ]);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      await flushPromises();

      expect(wrapper.text()).toContain("Recent Projects");
      const recent = wrapper.get('[data-testid="menu-item-recent:/Trabajo/skills"]');
      expect(recent.text()).toContain("skills");

      await recent.trigger("click");
      await flushPromises();

      expect(mocks.openPath).toHaveBeenCalledWith("/Trabajo/skills");
      wrapper.unmount();
    });

    it("offers no recents group when nothing has been opened before", async () => {
      mocks.listRecentPaths.mockResolvedValue([]);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      await flushPromises();

      expect(wrapper.text()).not.toContain("Recent Projects");
      wrapper.unmount();
    });

    it("opens the worktrees of the repo from the branch crumb, and creates one from the root", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one"), { ...checkout("checkout:two"), branch: "feature" }),
      );

      const rows = wrapper.findAll('[data-testid^="menu-item-worktree:"]');
      // A branch names one worktree, so the rows are the branches and nothing else: the path is
      // in each row's tooltip.
      expect(rows.map((row) => row.text())).toEqual(["main", "feature"]);

      await wrapper.get('[data-testid="menu-item-new-worktree"]').trigger("click");
      await flushPromises();

      const dialog = wrapper.get('[data-testid="worktree-dialog"]');
      expect(dialog.attributes()).toMatchObject({ "data-open": "true", "data-mode": "create" });
      // The worktree is added to the repo's root, which is not the worktree that was picked.
      expect(dialog.attributes("data-checkout")).toBe("checkout:one");
      wrapper.unmount();
    });

    it("has no branch crumb to open when the workdir is a plain folder", async () => {
      const plain: WorkspaceState = {
        repos: [
          {
            id: "repo:notes",
            kind: "plain",
            name: "notes",
            root: "/notes",
            checkouts: [{ ...checkout("checkout:one"), repoId: "repo:notes", path: "/notes", canonicalPath: "/notes" }],
            createdAt: "now",
            lastOpenedAt: "now",
          },
        ],
        activeCheckoutId: "checkout:one",
        activeSessionId: null,
      };
      const wrapper = await mountApp(plain);

      // The workdir crumb is still there to open, and the line has nothing to say a branch.
      expect(wrapper.find('[data-testid="repo-crumb"]').exists()).toBe(true);
      expect(wrapper.find('[data-testid="worktree-crumb"]').exists()).toBe(false);
      expect(wrapper.findAll('[data-testid^="menu-item-worktree:"]')).toHaveLength(0);
      wrapper.unmount();
    });

    it("opens the terminals of the workdir from the last crumb, and starts a new one", async () => {
      const wrapper = await mountApp(
        workspaceWith(
          checkout("checkout:one", [session("session:one", "Terminal 1"), session("session:two", "Neovim")]),
          checkout("checkout:two", [session("session:three", "Other", "checkout:two")]),
        ),
      );

      // Only the workdir's own terminals, not the ones of the checkout next to it.
      const rows = wrapper.findAll('[data-testid^="menu-item-session:"]');
      expect(rows.map((row) => row.text())).toEqual(["Terminal 1", "Neovim"]);

      await rows[0]!.trigger("click");
      await flushPromises();

      expect(mocks.workspaceRef?.value.activeSessionId).toBe("session:one");
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Terminal 1");

      // The action at the foot asks the backend for a shell in this same workdir. The crumb
      // keeps naming the session that is selected, which is the one the new shell joins.
      await wrapper.get('[data-testid="menu-item-new-terminal"]').trigger("click");
      await flushPromises();
      expect(mocks.selectCheckout).toHaveBeenCalledWith("checkout:one");
      wrapper.unmount();
    });

    it("names the open terminal the way the sidebar row names it", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one", [session("session:one", "zsh"), session("session:two", "Neovim")])),
      );

      await wrapper.get('[data-testid="menu-item-session:one"]').trigger("click");
      await flushPromises();

      // Nothing is in front of the shell yet, so the crumb is on the name the database holds.
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("zsh");

      wrapper.getComponent({ name: "SessionPane" }).vm.$emit("sessionStatusChanged", "session:one", {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: "OpenCode: review task",
      });
      await flushPromises();

      // The crumb and the row that opens it are one list read twice, so the title the program set
      // is what the header says as well — and the menu offers the names the sidebar is showing.
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("OpenCode: review task");
      expect(wrapper.get('[data-testid="item-crumb"]').attributes("title")).toBe("OpenCode: review task");
      expect(wrapper.findAll('[data-testid^="menu-item-session:"]').map((row) => row.text())).toEqual([
        "OpenCode: review task",
        "Neovim",
      ]);
      wrapper.unmount();
    });

    it("requests a shell for the launch checkout", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      mocks.launchCheckoutId!.value = "checkout:one";
      await flushPromises();

      expect(mocks.selectCheckout).toHaveBeenCalledTimes(1);
      expect(mocks.selectCheckout).toHaveBeenCalledWith("checkout:one");
      wrapper.unmount();
    });

    it("launches Home after restore when it has no terminal session", async () => {
      const homeCheckout = checkout("checkout:home");
      const homeRepo: Repo = {
        ...workspaceWith(homeCheckout).repos[0]!,
        kind: "plain",
        name: "Home",
      };
      mocks.restoreWorkspace.mockResolvedValue({
        repos: [homeRepo],
        activeCheckoutId: homeCheckout.id,
        activeSessionId: null,
        homeCheckoutId: homeCheckout.id,
      });
      const { useWorkspaceState } =
        await vi.importActual<typeof import("./presentation/workspace")>("./presentation/workspace");
      let launchCheckoutId: Ref<string | null> | undefined;
      const host = defineComponent({
        setup() {
          launchCheckoutId = useWorkspaceState().launchCheckoutId;
          return () => h("div");
        },
      });

      const wrapper = mount(host);
      await flushPromises();

      expect(mocks.restoreWorkspace).toHaveBeenCalledTimes(1);
      expect(launchCheckoutId?.value).toBe(homeCheckout.id);
      wrapper.unmount();
    });

    it("draws a file path as the steps it is made of", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      await wrapper.get('[data-testid="open-nested-file"]').trigger("click");
      await flushPromises();

      // A path is a line of crumbs already, so it reads as one: the steps with the separator
      // the rest of the line uses, and still nothing to click.
      const crumb = wrapper.get('[data-testid="item-crumb"]');
      expect(crumb.text()).toBe("src/lib/one.ts");
      expect(crumb.attributes("title")).toBe("src/lib/one.ts");
      expect(crumb.findAll("span[aria-hidden='true']").map((el) => el.text())).toEqual(["/", "/"]);
      expect(crumb.classes()).not.toContain("text-menu-control");
      wrapper.unmount();
    });

    it("writes a renamed session to the database and repaints from its answer", async () => {
      mocks.renameTerminal.mockResolvedValue(
        workspaceWith(checkout("checkout:one", [{ ...session("session:one", "build logs"), name: "build logs" }])),
      );
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      await wrapper.get('[data-testid="rename-session-one"]').trigger("click");
      await flushPromises();

      // The row is renamed against the checkout it is listed under, never against a name the
      // caller made up: the id and the name are the two things the command needs.
      expect(mocks.renameTerminal).toHaveBeenCalledWith("checkout:one", "session:one", "build logs");
      // The workspace comes back from the rename, so the sidebar paints the name the database
      // now holds rather than the one that was asked for.
      expect(mocks.workspaceRef?.value.activeCheckoutId).toBe("checkout:one");
      expect(mocks.workspaceRef?.value.repos[0]!.checkouts[0]!.sessions[0]!.name).toBe("build logs");
      wrapper.unmount();
    });

    it("reports a rename the backend refuses instead of painting it anyway", async () => {
      mocks.renameTerminal.mockRejectedValue(new Error("a session needs a name"));
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      await wrapper.get('[data-testid="rename-session-one"]').trigger("click");
      await flushPromises();

      // The row keeps the name it had, and the refusal is said out loud rather than swallowed.
      const { toasts } = useToasts();
      expect(toasts.value.at(-1)?.message).toContain("a session needs a name");
      wrapper.unmount();
    });

    it("drops the middle of the path when the line has no room for it", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      await wrapper.get('[data-testid="open-nested-file"]').trigger("click");
      await flushPromises();
      const crumb = () => wrapper.get('[data-testid="item-crumb"]');
      expect(crumb().text()).toBe("src/lib/one.ts");

      // The path at the width it wants, wider than any room the line could give it. The probe
      // is what is weighed, so this is the whole decision in one number.
      const probe = wrapper.get('[data-testid="path-probe"]').element;
      Object.defineProperty(probe, "scrollWidth", { configurable: true, value: 400 });
      window.dispatchEvent(new Event("resize"));
      await wrapper.vm.$nextTick();

      // What goes is the middle: the first directory says where you are, the file name is what
      // you came to see, and the tooltip still has all of it.
      expect(crumb().text()).toBe("src/…/one.ts");
      expect(
        crumb()
          .findAll("span[aria-hidden='true']")
          .map((el) => el.text()),
      ).toEqual(["/", "/"]);
      expect(crumb().attributes("title")).toBe("src/lib/one.ts");
      wrapper.unmount();
    });

    it("never elides a path with nothing in the middle to drop", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      await wrapper.get('[data-testid="open-file"]').trigger("click");
      await flushPromises();
      // One step has no middle, so there is nothing to shorten it by.
      expect(wrapper.find('[data-testid="path-probe"]').exists()).toBe(false);
      expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("README.md");
      wrapper.unmount();
    });

    it("leaves the last crumb out when there is no terminal to name", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      // A crumb that reads "Terminal" over a panel that says it has none is naming nothing, and
      // neither is the separator that would come before it.
      const separators = () => wrapper.findAll("nav > span[aria-hidden='true']");
      expect(wrapper.find('[data-testid="item-crumb"]').exists()).toBe(false);
      expect(separators()).toHaveLength(1);

      // A workdir that has one names it, and the sessions are what the menu then lists.
      const withSession = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));
      expect(withSession.get('[data-testid="item-crumb"]').text()).toBe("zsh");
      expect(withSession.findAll("nav > span[aria-hidden='true']")).toHaveLength(2);
      expect(withSession.find('[data-testid="menu-item-session:one"]').exists()).toBe(true);
      wrapper.unmount();
      withSession.unmount();
    });

    it("keeps one crumb menu open at a time", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      const roots = () => wrapper.findAllComponents({ name: "DropdownMenuRoot" });
      const openNames = () => roots().map((root) => root.attributes("data-open"));

      // Nothing opens on its own. Each root reports that it opened and the shell names the one
      // that is, so the three menus can never be stacked on the same header.
      expect(openNames()).toEqual(["false", "false", "false"]);
      await roots()[0]!.vm.$emit("update:open", true);
      await flushPromises();
      expect(openNames()).toEqual(["true", "false", "false"]);

      await roots()[1]!.vm.$emit("update:open", true);
      await flushPromises();
      expect(openNames()).toEqual(["false", "true", "false"]);
      wrapper.unmount();
    });

    it("offers a new terminal from the menu of a workdir that has one open", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));

      expect(wrapper.find('[data-testid="menu-item-session:one"]').exists()).toBe(true);
      expect(wrapper.find('[data-testid="menu-item-new-terminal"]').exists()).toBe(true);
      wrapper.unmount();
    });

    it("leaves the crumbs as text: no chip, no chevron", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

      for (const testid of ["repo-crumb", "worktree-crumb", "item-crumb"]) {
        const crumb = wrapper.get(`[data-testid="${testid}"]`);
        expect(crumb.classes()).toContain("text-menu-control");
        expect(crumb.classes()).not.toContain("marvis-control");
      }
      // The workdir is the one that says where you are, so it is the only crumb set forward, and
      // each crumb carries its own place in the line, which is what says what it gives up.
      expect(wrapper.get('[data-testid="repo-crumb"]').classes()).toContain("crumb-workdir");
      expect(wrapper.get('[data-testid="worktree-crumb"]').classes()).toContain("crumb-branch");
      expect(wrapper.get('[data-testid="item-crumb"]').classes()).toContain("crumb-item");

      // The glyph each crumb wears is the one its row wears in the sidebar, and it is the only
      // thing on the crumb but the name: no chip, no chevron, and nothing drawn between two
      // crumbs that belongs to neither of them.
      expect(wrapper.get('[data-testid="worktree-crumb"]').findAll(".crumb-icon")).toHaveLength(1);
      expect(wrapper.get('[data-testid="item-crumb"]').findAll(".crumb-icon")).toHaveLength(1);
      expect(wrapper.get('[data-testid="repo-crumb"]').findAll(".crumb-icon")).toHaveLength(0);
      wrapper.unmount();
    });

    it("names the settings gear for a screen reader", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      const settings = wrapper.get('[data-testid="settings-button"]');
      expect(settings.attributes("aria-label")).toBe("Settings");
      expect(settings.find("svg").exists()).toBe(true);
      // The dialog itself is asserted in its own block; what belongs here is that the gear opens
      // one at all rather than sitting there.
      await settings.trigger("click");
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
      wrapper.unmount();
    });

    it("renders no error banner", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      expect(wrapper.find('[role="alert"]').exists()).toBe(false);
      wrapper.unmount();
    });
  });

  describe("a checkout whose directory is gone", () => {
    it("confirms the scope each close reaches, and closes only what was confirmed", async () => {
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two")));

      // The repo root is the head of a list, so closing it reaches the worktrees with it.
      await wrapper.get('[data-testid="close-missing-base"]').trigger("click");
      expect(confirm).toHaveBeenLastCalledWith(
        "Close “/checkout:one” and its checkout list in Marvis? No files will be deleted.",
      );
      // A worktree is one entry, and nothing is closed while the question is unanswered.
      await wrapper.get('[data-testid="close-missing-worktree"]').trigger("click");
      expect(confirm).toHaveBeenLastCalledWith("Close “/checkout:two” in Marvis? No files will be deleted.");
      expect(mocks.closeMissingCheckout).not.toHaveBeenCalled();

      confirm.mockReturnValue(true);
      mocks.closeMissingCheckout.mockResolvedValue({ repos: [], activeCheckoutId: null, activeSessionId: null });
      await wrapper.get('[data-testid="close-missing-worktree"]').trigger("click");
      await flushPromises();

      expect(mocks.closeMissingCheckout).toHaveBeenCalledWith("checkout:two");
      expect(mocks.workspaceRef?.value.repos).toEqual([]);
      wrapper.unmount();
      confirm.mockRestore();
    });

    it("names a plain folder as losing its checkout list as well", async () => {
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      const plain: WorkspaceState = {
        repos: [
          {
            id: "repo:notes",
            kind: "plain",
            name: "notes",
            root: "/notes",
            checkouts: [
              {
                // A plain folder is one primary checkout, so the stub's base entry is it.
                ...checkout("checkout:one"),
                repoId: "repo:notes",
                path: "/notes",
                canonicalPath: "/notes",
                branch: undefined,
              },
            ],
            createdAt: "now",
            lastOpenedAt: "now",
          },
        ],
        activeCheckoutId: "checkout:one",
        activeSessionId: null,
      };
      mocks.closeMissingCheckout.mockResolvedValue({ repos: [], activeCheckoutId: null, activeSessionId: null });
      const wrapper = await mountApp(plain);

      await wrapper.get('[data-testid="close-missing-base"]').trigger("click");

      expect(confirm).toHaveBeenCalledWith("Close “/notes” and its checkout list in Marvis? No files will be deleted.");
      expect(mocks.closeMissingCheckout).toHaveBeenCalledWith("checkout:one");
      wrapper.unmount();
      confirm.mockRestore();
    });
  });

  describe("a workdir with its directory still there", () => {
    it("asks first, names the scope, and only then takes the row off the list", async () => {
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two")));

      // The repo root is the head of a list, so its row's close reaches the worktrees with it.
      await wrapper.get('[data-testid="close-workdir-base"]').trigger("click");
      expect(confirm).toHaveBeenLastCalledWith(
        "Remove “/checkout:one” and its checkout list from Marvis? No files will be deleted, and opening the folder again brings it back.",
      );
      // A worktree is one entry, and nothing is closed while the question is unanswered.
      await wrapper.get('[data-testid="close-workdir-worktree"]').trigger("click");
      expect(confirm).toHaveBeenLastCalledWith(
        "Remove “/checkout:two” from Marvis? No files will be deleted, and opening the folder again brings it back.",
      );
      expect(mocks.closeCheckout).not.toHaveBeenCalled();

      confirm.mockReturnValue(true);
      mocks.closeCheckout.mockResolvedValue({ repos: [], activeCheckoutId: null, activeSessionId: null });
      await wrapper.get('[data-testid="close-workdir-worktree"]').trigger("click");
      await flushPromises();

      expect(mocks.closeCheckout).toHaveBeenCalledWith("checkout:two");
      expect(mocks.workspaceRef?.value.repos).toEqual([]);
      wrapper.unmount();
      confirm.mockRestore();
    });

    it("never reaches the missing-checkout command, which prunes what a live workdir still needs", async () => {
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      mocks.closeCheckout.mockResolvedValue({ repos: [], activeCheckoutId: null, activeSessionId: null });
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two")));

      await wrapper.get('[data-testid="close-workdir-base"]').trigger("click");
      await flushPromises();

      expect(mocks.closeMissingCheckout).not.toHaveBeenCalled();
      wrapper.unmount();
      confirm.mockRestore();
    });
  });

  describe("archiving a worktree and putting it back", () => {
    const archived = [
      { id: "checkout:three", repoId: "repo:shared", path: "/checkout:three", branch: "three" },
      { id: "checkout:four", repoId: "repo:shared", path: "/checkout:four", branch: "four" },
    ];

    it("opens one dialog for both answers when the row asks to take the worktree off the panel", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one"), checkout("checkout:two")));

      await wrapper.get('[data-testid="remove-worktree-two"]').trigger("click");

      // Archiving and deleting are one question, so the row asks it once and the dialog
      // answers with both, against the worktree that was clicked.
      const dialog = wrapper.get('[data-testid="worktree-dialog"]');
      expect(dialog.attributes("data-mode")).toBe("remove");
      expect(dialog.attributes("data-checkout")).toBe("checkout:two");
      wrapper.unmount();
    });

    it("restores a whole repository's archived worktrees in one confirmed action", async () => {
      mocks.restoreArchivedWorktrees.mockResolvedValue({
        ...workspaceWith(checkout("checkout:one"), checkout("checkout:two"), checkout("checkout:three")),
        archivedWorktrees: [],
      });
      const wrapper = await mountApp({ ...workspaceWith(checkout("checkout:one")), archivedWorktrees: archived });

      await wrapper.get('[data-testid="restore-archived-shared"]').trigger("click");

      // Two, said as two: the count is what the user is about to see change.
      const dialog = wrapper.get('[role="dialog"]');
      expect(dialog.text()).toContain("Restore archived worktrees");
      expect(dialog.text()).toContain("2 archived worktrees");
      expect(mocks.restoreArchivedWorktrees).not.toHaveBeenCalled();

      await dialog.findAll("footer button")[0]!.trigger("click");
      expect(mocks.restoreArchivedWorktrees).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("says one when there is only one, so the row never promises a set of none", async () => {
      const wrapper = await mountApp({
        ...workspaceWith(checkout("checkout:one")),
        archivedWorktrees: [archived[0]!],
      });

      await wrapper.get('[data-testid="restore-archived-shared"]').trigger("click");
      await flushPromises();

      expect(wrapper.get('[role="dialog"]').text()).toContain("Restore archived worktree");
      expect(wrapper.get('[role="dialog"]').text()).toContain("1 archived worktree");
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
        version: 5,
        mode: "focus",
        sidebarWidth: 310,
        inspectorWidth: 340,
        previewWidth: 360,
      });
      wrapper.unmount();
    });

    it("clamps a drag that goes past the supported range", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [900, 400, 12]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();

      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 5,
        mode: "focus",
        sidebarWidth: 500,
        inspectorWidth: 200,
        previewWidth: 360,
      });
      wrapper.unmount();
    });

    it("keeps the inspector width when the narrow drawer reports a collapsed panel", async () => {
      // The scrollbar mode is carried through the same save rather than checked on its own, so
      // this one is stored as `always`: a preference read back as the default would be invisible
      // everywhere else, because the default is what a layout with no preference in it gives.
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 5,
        mode: "focus",
        sidebarWidth: 345,
        inspectorWidth: 450,
        previewWidth: 360,
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
        version: 5,
        mode: "focus",
        sidebarWidth: 400,
        inspectorWidth: 450,
        previewWidth: 360,
      });
      wrapper.unmount();
    });

    it("lets double-click on a handle reset just that panel", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 5,
        mode: "focus",
        sidebarWidth: 345,
        inspectorWidth: 450,
        previewWidth: 360,
      });
      const resizeCalls = vi.fn();
      mocks.onProgrammaticPanelResize = resizeCalls;

      const handles = wrapper.findAllComponents(SplitterResizeHandle);
      const grip = handles[0]!.find(".h-6.w-1");
      expect(grip.exists()).toBe(true);
      expect(grip.classes().some((className) => className.startsWith("rounded"))).toBe(false);
      await handles[0]!.trigger("dblclick");
      await flushPromises();

      expect(resizeCalls).toHaveBeenCalledWith("navigation-panel", DEFAULT_APP_LAYOUT.sidebarWidth);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();
      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        version: 5,
        mode: "focus",
        sidebarWidth: DEFAULT_APP_LAYOUT.sidebarWidth,
        inspectorWidth: 450,
        previewWidth: 360,
      });
      wrapper.unmount();
    });

    it("gives the inspector its full width back when space returns", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        version: 5,
        mode: "focus",
        sidebarWidth: 300,
        inspectorWidth: 300,
        previewWidth: 360,
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

    it("keeps the terminal mounted with the latest preview in split mode", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

      await wrapper.get('[data-testid="open-file"]').trigger("click");
      await wrapper.get('[data-testid="layout-toggle"]').trigger("click");
      expect(wrapper.get('[data-testid="layout-toggle"]').attributes()).toMatchObject({
        "aria-label": "Switch to focus layout",
        "aria-pressed": "true",
      });
      expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).not.toBe("none");
      expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");
      expect(mocks.sessionPaneMounts).toBe(1);

      const resizeHandle = wrapper.get('[aria-label="Resize preview panel"]');
      await resizeHandle.trigger("pointermove", { pointerId: 1, clientX: 400 });
      expect(wrapper.getComponent({ name: "MainPane" }).emitted("resizePreview")).toBeUndefined();
      await resizeHandle.trigger("keydown", { key: "ArrowLeft" });
      await wrapper.get('[data-testid="select-session-one"]').trigger("click");
      await flushPromises();
      expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");
      expect(mocks.sessionPaneMounts).toBe(1);

      await wrapper.get('[data-testid="new-terminal-one"]').trigger("click");
      await flushPromises();
      expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");

      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();
      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({ ...DEFAULT_APP_LAYOUT, mode: "split", previewWidth: 380 });
      expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
        "checkout:one",
        expect.objectContaining({ mainView: "document", document: expect.objectContaining({ path: "README.md" }) }),
      );
      await wrapper.get('[data-testid="layout-toggle"]').trigger("click");
      expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).toBe("none");
      expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");
      wrapper.unmount();
    });

    it("resizes the preview only while its handle is being dragged", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        ...DEFAULT_APP_LAYOUT,
        mode: "split",
      });
      await wrapper.get('[data-testid="open-file"]').trigger("click");
      const handle = wrapper.get('[aria-label="Resize preview panel"]');
      const pane = wrapper.getComponent({ name: "MainPane" });
      const element = handle.element as HTMLElement;
      element.setPointerCapture = vi.fn();
      element.hasPointerCapture = vi.fn().mockReturnValue(true);
      element.releasePointerCapture = vi.fn();
      vi.spyOn(wrapper.get("#main-panel > div").element, "getBoundingClientRect").mockReturnValue({
        right: 1000,
      } as DOMRect);

      await handle.trigger("pointermove", { pointerId: 1, clientX: 700 });
      expect(pane.emitted("resizePreview")).toBeUndefined();
      await handle.trigger("pointerdown", { pointerId: 1, clientX: 700 });
      await handle.trigger("pointermove", { pointerId: 2, clientX: 650 });
      expect(pane.emitted("resizePreview")).toBeUndefined();
      await handle.trigger("pointermove", { pointerId: 1, clientX: 650 });
      expect(pane.emitted("resizePreview")).toEqual([[345]]);
      await handle.trigger("pointerup", { pointerId: 1 });
      await handle.trigger("pointermove", { pointerId: 1, clientX: 600 });
      expect(pane.emitted("resizePreview")).toHaveLength(1);
      wrapper.unmount();
    });

    it("peeks the split inspector for 300ms and closes it with Escape", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), {
        ...DEFAULT_APP_LAYOUT,
        mode: "split",
      });
      const inspector = wrapper.findComponent({ name: "InspectorPane" });

      expect(wrapper.find(".inspector-hover-strip").exists()).toBe(true);
      expect(inspector.element.parentElement).toBe(wrapper.get(".app-shell").element);
      expect(inspector.classes()).toContain("split-inspector-closed");
      await wrapper.get(".inspector-hover-strip").trigger("pointerenter");
      expect(inspector.classes()).not.toContain("split-inspector-closed");

      await wrapper.get(".inspector-hover-strip").trigger("pointerleave");
      await vi.advanceTimersByTimeAsync(299);
      expect(inspector.classes()).not.toContain("split-inspector-closed");
      await inspector.trigger("pointerenter");
      await vi.advanceTimersByTimeAsync(300);
      expect(inspector.classes()).not.toContain("split-inspector-closed");

      await inspector.trigger("pointerleave");
      await vi.advanceTimersByTimeAsync(299);
      expect(inspector.classes()).not.toContain("split-inspector-closed");
      await inspector.trigger("pointerenter");
      await vi.advanceTimersByTimeAsync(300);
      expect(inspector.classes()).not.toContain("split-inspector-closed");

      // The drawer appearing under the pointer fires a leave on the strip and an enter on the
      // drawer at once. In whichever order the browser delivers them the drawer stays open, and
      // it closes on the single leave that means the pointer is over neither.
      await wrapper.get(".inspector-hover-strip").trigger("pointerenter");
      await inspector.trigger("pointerenter");
      await wrapper.get(".inspector-hover-strip").trigger("pointerleave");
      await vi.advanceTimersByTimeAsync(300);
      expect(inspector.classes()).not.toContain("split-inspector-closed");

      await inspector.trigger("pointerleave");
      await vi.advanceTimersByTimeAsync(300);
      expect(inspector.classes()).toContain("split-inspector-closed");
      await wrapper.get(".inspector-hover-strip").trigger("pointerenter");
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
      await flushPromises();
      expect(inspector.classes()).toContain("split-inspector-closed");

      Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();
      expect(wrapper.find(".inspector-hover-strip").exists()).toBe(true);
      expect(inspector.classes()).toContain("right-inspector-drawer");
      expect(inspector.classes()).toContain("split-inspector-closed");
      expect(wrapper.findComponent({ name: "MainPane" }).props("split")).toBe(true);
      wrapper.unmount();
    });

    it("forgets a drawer hovered under another layout, so it can still be closed", async () => {
      // The drawer is the same element in the narrow focus layout and in the split one, so a
      // pointerenter while focused says nothing about a strip that does not exist yet. Carried
      // over, the flag refuses every close: the drawer is hidden, a hidden drawer fires no leave,
      // and the panel stays up over the split layout with nothing on screen able to put it away.
      Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
      window.dispatchEvent(new Event("resize"));
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")), { ...DEFAULT_APP_LAYOUT });
      // Unmounted in a finally because a failed assertion in here used to leave a live tree behind,
      // and the drawers its watchers answer leaked into the layout tests that follow.
      try {
        const inspector = wrapper.findComponent({ name: "InspectorPane" });
        expect(inspector.classes()).toContain("right-inspector-drawer");

        await inspector.trigger("pointerenter");
        await wrapper.get('[data-testid="layout-toggle"]').trigger("click");
        await flushPromises();
        expect(wrapper.findComponent({ name: "MainPane" }).props("split")).toBe(true);

        await wrapper.get(".inspector-hover-strip").trigger("pointerenter");
        await wrapper.get(".inspector-hover-strip").trigger("pointerleave");
        await vi.advanceTimersByTimeAsync(300);
        expect(wrapper.findComponent({ name: "InspectorPane" }).classes()).toContain("split-inspector-closed");
      } finally {
        wrapper.unmount();
      }
    });

    it("does not bring a drawer back open when the shortcut hides it", async () => {
      // ⌘B is the only way to put the drawer away without a pointer over it, so it is also the
      // only way to strand the flag that refuses every close: a hidden drawer fires no leave.
      // Attached and dispatched from a panel, because that is the path the capture listener on the
      // window actually answers to.
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT, mode: "split" },
        { attachTo: document.body },
      );
      try {
        const press = () =>
          wrapper
            .get("#navigation-panel")
            .element.dispatchEvent(new KeyboardEvent("keydown", { key: "b", metaKey: true, bubbles: true }));
        const inspector = wrapper.findComponent({ name: "InspectorPane" });
        await wrapper.get(".inspector-hover-strip").trigger("pointerenter");
        expect(inspector.classes()).not.toContain("split-inspector-closed");

        expect(press()).toBe(true); // not cancelled: the event was handled
        await flushPromises();
        expect(inspector.isVisible()).toBe(false);

        press();
        await flushPromises();
        expect(inspector.isVisible()).toBe(true);
        expect(inspector.classes()).toContain("split-inspector-closed");
      } finally {
        wrapper.unmount();
      }
    });
  });

  describe("the window scale", () => {
    const press = (init: KeyboardEventInit) => {
      const event = new KeyboardEvent("keydown", { cancelable: true, ...init });
      window.dispatchEvent(event);
      return event;
    };
    const scale = () => document.documentElement.style.getPropertyValue("zoom");
    const { toasts } = useToasts();
    const settingsAtZoom = (zoom: number): AppSettings => ({
      ...cloneSettings(DEFAULT_SETTINGS),
      ui: { ...DEFAULT_SETTINGS.ui, zoom: zoom as AppSettings["ui"]["zoom"] },
    });

    it("scales the window one step and says where it landed", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      expect(press({ key: "+", metaKey: true }).defaultPrevented).toBe(true);
      await flushPromises();

      expect(scale()).toBe("1.1");
      expect(toasts.value.map((toast) => toast.message)).toEqual(["Zoom 110% (Cmd 0 to reset)"]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();
      expect(mocks.saveSettings).toHaveBeenLastCalledWith(settingsAtZoom(1.1));
      wrapper.unmount();
    });

    it("does not let an older failed write roll back a newer zoom step", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
      let rejectFirst!: (error: Error) => void;
      let resolveSecond!: () => void;
      mocks.saveSettings
        .mockImplementationOnce(() => new Promise<void>((_, reject) => (rejectFirst = reject)))
        .mockImplementationOnce(() => new Promise<void>((resolve) => (resolveSecond = resolve)));

      press({ key: "+", metaKey: true });
      press({ key: "+", metaKey: true });
      await flushPromises();
      expect(scale()).toBe("1.2");
      expect(mocks.saveSettings).toHaveBeenCalledTimes(1);

      rejectFirst(new Error("older write failed"));
      await flushPromises();
      expect(mocks.saveSettings).toHaveBeenCalledTimes(2);
      expect(scale()).toBe("1.2");

      resolveSecond();
      await flushPromises();
      expect(scale()).toBe("1.2");
      expect(mocks.saveSettings).toHaveBeenLastCalledWith(settingsAtZoom(1.2));
      expect(toasts.value.at(-1)?.message).toBe("Zoom 120% (Cmd 0 to reset)");
      wrapper.unmount();
    });

    it("names Ctrl when Ctrl is the key that worked", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      press({ key: "=", ctrlKey: true });
      await flushPromises();

      expect(scale()).toBe("1.1");
      expect(toasts.value.at(-1)?.message).toBe("Zoom 110% (Ctrl 0 to reset)");
      wrapper.unmount();
    });

    it("goes back to 100% on 0, from wherever it was", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      press({ key: "+", metaKey: true });
      press({ key: "+", metaKey: true });
      press({ key: "-", metaKey: true });
      press({ key: "0", metaKey: true });
      await flushPromises();

      expect(scale()).toBe("1");
      expect(toasts.value.at(-1)?.message).toBe("Zoom 100% (Cmd 0 to reset)");
      wrapper.unmount();
    });

    it("keeps one line of feedback while the key is held down", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      for (let step = 0; step < 3; step += 1) {
        press({ key: "+", metaKey: true });
        await flushPromises();
      }

      // Three different numbers, and `push` only folds away a message identical to the one before
      // it, so without taking the last one down the stack would be a history of the gesture.
      expect(toasts.value.map((toast) => toast.message)).toEqual(["Zoom 130% (Cmd 0 to reset)"]);
      wrapper.unmount();
    });

    it("says nothing when the key cannot move the scale any further", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { settings: { ...structuredClone(DEFAULT_SETTINGS), ui: { ...DEFAULT_SETTINGS.ui, zoom: 1.5 } } },
      );
      expect(scale()).toBe("1.5");

      press({ key: "+", metaKey: true });
      await flushPromises();

      expect(scale()).toBe("1.5");
      expect(toasts.value).toEqual([]);
      wrapper.unmount();
    });

    it("says nothing when a 0 arrives at 100%", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      press({ key: "0", metaKey: true });
      await flushPromises();

      expect(scale()).toBe("1");
      expect(toasts.value).toEqual([]);
      wrapper.unmount();
    });

    it("leaves a 0 and a + that are being typed alone", async () => {
      const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

      press({ key: "0" });
      press({ key: "+" });
      press({ key: "a", metaKey: true });
      await flushPromises();

      expect(scale()).toBe("1");
      expect(toasts.value).toEqual([]);
      expect(mocks.saveAppLayout).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("comes back at the scale it was left at, and lays the window out in that space", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { settings: { ...structuredClone(DEFAULT_SETTINGS), ui: { ...DEFAULT_SETTINGS.ui, zoom: 1.5 } } },
      );

      expect(scale()).toBe("1.5");
      expect(wrapper.findComponent({ name: "MainPane" }).props("zoom")).toBe(1.5);
      // The app arranges itself in the space the window has before the scale, and a window 1400
      // wide at 150% is 933 of that, which is not enough for the panels beside the main one.
      expect(wrapper.get("#inspector-panel").classes()).toContain("min-h-0");
      expect(wrapper.findComponent({ name: "InspectorPane" }).classes()).toContain("right-inspector-drawer");

      press({ key: "0", metaKey: true });
      await flushPromises();
      expect(wrapper.findComponent({ name: "InspectorPane" }).classes()).not.toContain("right-inspector-drawer");
      wrapper.unmount();
    });

    it("stores the panel widths in the app's pixels rather than the window's", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { settings: { ...structuredClone(DEFAULT_SETTINGS), ui: { ...DEFAULT_SETTINGS.ui, zoom: 1.2 } } },
      );
      const resizeCalls = vi.fn();
      mocks.onProgrammaticPanelResize = resizeCalls;

      // The splitter measures with `getBoundingClientRect()`, which is a number after the scale.
      wrapper.getComponent(SplitterGroup).vm.$emit("layout", [300, 700, 300]);
      await vi.advanceTimersByTimeAsync(300);
      await flushPromises();
      expect(mocks.saveAppLayout).toHaveBeenLastCalledWith({
        ...DEFAULT_APP_LAYOUT,
        sidebarWidth: 250,
        inspectorWidth: 250,
      });

      // And a reset is a width in the same units, so it goes into the splitter scaled.
      const handles = wrapper.findAllComponents(SplitterResizeHandle);
      await handles[0]!.trigger("dblclick");
      await flushPromises();
      expect(resizeCalls).toHaveBeenCalledWith("navigation-panel", DEFAULT_APP_LAYOUT.sidebarWidth * 1.2);
      wrapper.unmount();
    });
  });

  it("exports a review by default without dispatching a round or changing note status", async () => {
    mocks.gitStatus = { branch: "feature", defaultBranch: "main" };
    mocks.reviewNotes = [reviewNoteFixture()];
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));

    await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();

    expect(mocks.exportReviewMarkdown).toHaveBeenCalledOnce();
    const [date, timestamp, markdown] = mocks.exportReviewMarkdown.mock.calls[0];
    expect(date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(timestamp).toMatch(new RegExp(`^${date}-\\d{4}$`));
    expect(markdown).toContain(`# Code Review ${date}`);
    expect(mocks.dispatchReviewRound).not.toHaveBeenCalled();
    expect(mocks.createAgentSession).not.toHaveBeenCalled();
    expect(mocks.reviewNotes[0]?.status).toBe("draft");
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("2026-03-14-1532.md");

    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
      "checkout:one",
      expect.objectContaining({ document: expect.objectContaining({ origin: "review" }) }),
    );
    wrapper.unmount();
  });

  it("dispatches the review to the chosen agent session as one round", async () => {
    mocks.loadReviewTarget.mockResolvedValue("opencode");
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

    // The diff is the view that sends, so the review goes out from there.
    await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
    await flushPromises();
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
    mocks.loadReviewTarget.mockResolvedValue("opencode");
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

    await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();

    expect(mocks.createAgentSession).toHaveBeenCalledTimes(1);
    expect(mocks.dispatchReviewRound.mock.calls[0][0]).toBe("ses_new");
    wrapper.unmount();
  });

  it("switches the main view from the inspector and back from a crumb, without unmounting the terminal", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

    // The whole change set, then one file's diff: both are the diff view.
    await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-diff").element as HTMLElement).style.display).not.toBe("none");
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("All changes");
    await wrapper.get('[data-testid="diff-scroll"]').trigger("click");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
      "checkout:one",
      expect.objectContaining({ diffScrollTop: 132, diffAllFiles: true, document: null }),
    );

    await wrapper.get('[data-testid="open-file"]').trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-diff").element as HTMLElement).style.display).toBe("none");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).not.toBe("none");
    // The crumb names the view that is open, not the session behind it.
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("README.md");
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("README.md");

    await wrapper.get('[data-testid="document-scroll"]').trigger("click");
    await wrapper.get('[data-testid="changes-scroll"]').trigger("click");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
      "checkout:one",
      expect.objectContaining({
        mainView: "document",
        document: { checkoutId: "checkout:one", path: "README.md", origin: "checkout", source: "file", mode: "view" },
        documentScrollTop: 240,
        documentScrollLeft: 12,
        // A new view starts at the top, so the diff offset does not follow the document.
        diffScrollTop: 0,
        changesScrollTop: 84,
      }),
    );

    // Back to the terminal from the sidebar: the last crumb's menu only exists while a terminal
    // is what the panel is showing, so a file leaves it as plain text with nothing to open.
    expect(wrapper.get('[data-testid="item-crumb"]').classes()).not.toContain("text-menu-control");
    expect(wrapper.findAll('[data-testid^="menu-item-session:"]')).toHaveLength(0);
    await wrapper.get('[data-testid="select-session-one"]').trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).not.toBe("none");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).toBe("none");
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("Terminal 1");
    expect(mocks.sessionPaneMounts).toBe(1);
    wrapper.unmount();
  });

  it("saves the whole change set as the checkout's restored view", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "Terminal 1")])));

    await wrapper.get('[data-testid="open-all-changes"]').trigger("click");
    await flushPromises();
    expect((wrapper.get("#main-view-diff").element as HTMLElement).style.display).not.toBe("none");
    expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).toBe("none");
    expect(wrapper.get('[data-testid="file-diff"]').text()).toBe("all");
    expect(wrapper.get('[data-testid="item-crumb"]').text()).toBe("All changes");
    expect(mocks.sessionPaneMounts).toBe(1);

    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(mocks.saveCheckoutUiState).toHaveBeenLastCalledWith(
      "checkout:one",
      expect.objectContaining({ mainView: "terminal", diffAllFiles: true, document: null }),
    );
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
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("");

    resolveOne({
      ...DEFAULT_CHECKOUT_UI_STATE,
      document: { checkoutId: "checkout:one", path: "one.md", source: "file", mode: "view" },
      mainView: "document",
    });
    await flushPromises();
    expect(wrapper.get('[data-testid="document-pane"]').text()).toBe("");
    // The late load of the checkout left behind is dropped: the view that is open is the one
    // the active checkout was switched to.
    expect((wrapper.get("#main-view-terminal").element as HTMLElement).style.display).not.toBe("none");
    expect(mocks.saveCheckoutUiState).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("asks before stopping a running process, and cancelling leaves the window open", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "web")])));
    wrapper.getComponent({ name: "SessionPane" }).vm.$emit("sessionStatusChanged", "session:one", {
      state: "running",
      foregroundProcess: true,
      foregroundApp: "pnpm",
    });
    await flushPromises();
    mocks.getTerminalStatus.mockResolvedValue({
      state: "running",
      foregroundProcess: true,
      foregroundApp: "pnpm",
    });

    const closing = mocks.onCloseRequested!({ preventDefault: vi.fn() });
    await flushPromises();
    // Asked before anything is written or swept, and about the terminal the way the sidebar names
    // it — which here is the program, because nothing has renamed the terminal.
    const dialog = wrapper.findAll('[role="dialog"]').at(-1)!;
    expect(dialog.text()).toContain("pnpm");
    // And the program is not repeated beside a name that already is the program.
    expect(dialog.text()).not.toContain("(pnpm)");
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    expect(mocks.prepareAppExit).not.toHaveBeenCalled();

    await dialog.findAll("footer button").at(0)!.trigger("click");
    await Promise.all([closing, flushPromises()]);
    // Cancelled: nothing stopped, nothing written, and the window is still there to try again.
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    expect(mocks.prepareAppExit).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("closes without asking when every terminal is a shell at a prompt", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one", [session("session:one", "zsh")])));
    wrapper.getComponent({ name: "SessionPane" }).vm.$emit("sessionStatusChanged", "session:one", {
      state: "running",
      foregroundProcess: false,
    });
    await flushPromises();
    mocks.getTerminalStatus.mockResolvedValue({ state: "running", foregroundProcess: false });

    const closing = mocks.onCloseRequested!({ preventDefault: vi.fn() });
    await Promise.all([closing, flushPromises()]);
    // A shell at a prompt loses nothing, so there is nothing to ask about.
    expect(wrapper.findAll('[role="dialog"]')).toHaveLength(0);
    expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("waits for queued UI and settings writes before allowing a native close", async () => {
    const wrapper = await mountApp(workspaceWith(checkout("checkout:one")));
    let resolveLayoutWrite!: () => void;
    let resolveFirstSettingsWrite!: () => void;
    let resolveSecondSettingsWrite!: () => void;
    mocks.saveAppLayout.mockImplementation(() => new Promise<void>((resolve) => (resolveLayoutWrite = resolve)));
    mocks.saveSettings
      .mockImplementationOnce(() => new Promise<void>((resolve) => (resolveFirstSettingsWrite = resolve)))
      .mockImplementationOnce(() => new Promise<void>((resolve) => (resolveSecondSettingsWrite = resolve)));
    wrapper.getComponent(SplitterGroup).vm.$emit("layout", [320, 700, 300]);
    await vi.advanceTimersByTimeAsync(300);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "+", metaKey: true, cancelable: true }));
    await flushPromises();
    expect(mocks.saveAppLayout).toHaveBeenCalled();
    expect(mocks.saveSettings).toHaveBeenCalled();

    const preventDefault = vi.fn();
    const closeHandler = mocks.onCloseRequested!;
    const closing = closeHandler({ preventDefault });
    const duplicateClose = closeHandler({ preventDefault: vi.fn() });
    await flushPromises();
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();

    // A later preference action can arrive while the prevented close is flushing earlier work.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "+", metaKey: true, cancelable: true }));
    await flushPromises();
    resolveLayoutWrite();
    await flushPromises();
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    resolveFirstSettingsWrite();
    await flushPromises();
    expect(mocks.saveSettings).toHaveBeenCalledTimes(2);
    expect(mocks.currentWindow!.close).not.toHaveBeenCalled();
    resolveSecondSettingsWrite();
    await Promise.all([closing, duplicateClose]);
    expect(mocks.currentWindow!.close).toHaveBeenCalledOnce();
    wrapper.unmount();
  });
  describe("the sidebar shortcut", () => {
    const pressInSidebar = (init: KeyboardEventInit, target = wrapper.get("#navigation-panel").element) => {
      const event = new KeyboardEvent("keydown", { cancelable: true, bubbles: true, ...init });
      target.dispatchEvent(event);
      return event;
    };
    let wrapper: Awaited<ReturnType<typeof mountApp>>;

    beforeEach(async () => {
      // Attached, because the assertion below is about where a keydown stops: a tree that is not in
      // the document has no capture path from a panel to the window at all.
      wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { attachTo: document.body },
      );
    });

    it("collapses and brings back the navigation panel on Cmd+B", async () => {
      const panel = wrapper.findAllComponents({ name: "SplitterPanel" })[0]!;

      expect(pressInSidebar({ key: "b", metaKey: true }).defaultPrevented).toBe(true);
      await flushPromises();
      expect(panel.emitted("collapse")).toBeTruthy();

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(panel.emitted("expand")).toBeTruthy();
      wrapper.unmount();
    });

    it("takes the files and changes panel with it, so the main view is alone in the window", async () => {
      // One shortcut, both panels: a file tree and a change list standing between the main view and
      // the window's edge is the arrangement nobody asked for by hiding the sidebar.
      const panel = wrapper.findAllComponents({ name: "SplitterPanel" })[2]!;
      const inspector = () => wrapper.findComponent({ name: "InspectorPane" });

      expect(inspector().isVisible()).toBe(true);
      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(panel.emitted("collapse")).toBeTruthy();
      expect(inspector().isVisible()).toBe(false);

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(panel.emitted("expand")).toBeTruthy();
      expect(inspector().isVisible()).toBe(true);
      wrapper.unmount();
    });

    it("takes the handles away with the panels, so a hidden sidebar has nothing to grab", async () => {
      const handle = (id: string) => wrapper.findAll(id)[0];

      expect(handle("#navigation-resize-handle").isVisible()).toBe(true);
      expect(handle("#inspector-resize-handle").isVisible()).toBe(true);
      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(handle("#navigation-resize-handle").exists() && handle("#navigation-resize-handle").isVisible()).toBe(
        false,
      );
      expect(handle("#inspector-resize-handle").exists() && handle("#inspector-resize-handle").isVisible()).toBe(false);
      wrapper.unmount();
    });

    it("hides the drawer as well, in the two arrangements where the inspector floats", async () => {
      // Narrow and split both put the inspector over the main view, and a drawer has no width to
      // collapse: what shows it is the state it is bound to, which is the state the shortcut moved.
      // One arrangement at a time, because the key travels to the window and every app mounted on
      // this document answers it.
      for (const arrangement of [
        { width: 900, layout: { ...DEFAULT_APP_LAYOUT } },
        { width: 1400, layout: { ...DEFAULT_APP_LAYOUT, mode: "split" as const } },
      ]) {
        Object.defineProperty(window, "innerWidth", { configurable: true, value: arrangement.width });
        const floating = await mountApp(workspaceWith(checkout("checkout:one")), arrangement.layout, {
          attachTo: document.body,
        });

        expect(floating.findComponent({ name: "InspectorPane" }).isVisible()).toBe(true);
        pressInSidebar({ key: "b", metaKey: true }, floating.get("#navigation-panel").element);
        await flushPromises();
        expect(floating.findComponent({ name: "InspectorPane" }).isVisible()).toBe(false);
        // The strip is the drawer peeking on hover, and a strip that opens nothing is a hotspot
        // for a panel that is not there.
        expect(floating.find(".inspector-hover-strip").exists()).toBe(false);
        floating.unmount();
      }
      wrapper.unmount();
    });

    it("gives no width back to a panel that is still hidden", async () => {
      // A window that grows back is not a request to show the panel: an expanded panel nobody can
      // see is width the main view loses, so the shortcut is the only thing that brings it back.
      const resizeCalls = vi.fn();
      mocks.onProgrammaticPanelResize = resizeCalls;
      resizeCalls.mockClear();

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();
      Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
      window.dispatchEvent(new Event("resize"));
      await flushPromises();
      expect(resizeCalls).not.toHaveBeenCalled();

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(wrapper.findComponent({ name: "InspectorPane" }).isVisible()).toBe(true);
      wrapper.unmount();
    });

    it("stops the key before the terminal or the editor can read it as input", async () => {
      // The whole reason the listener is on capture: by the time a keydown reaches the window on
      // its way up, xterm has already turned it into input, and a shell reading Cmd+B as
      // backwards-char is the failure this shortcut exists to prevent.
      const seenByTerminal: string[] = [];
      (wrapper.get("#navigation-panel").element as HTMLElement).addEventListener("keydown", (event) => {
        seenByTerminal.push((event as KeyboardEvent).key);
      });

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();

      expect(seenByTerminal).toEqual([]);
      wrapper.unmount();
    });

    it("stops listening once the window it belongs to is gone", async () => {
      const removed = vi.spyOn(window, "removeEventListener");

      wrapper.unmount();

      // The capture flag is part of the registration: a listener removed without it is not the one
      // that was added, and this one would outlive the window and keep swallowing Cmd+B.
      expect(removed).toHaveBeenCalledWith("keydown", expect.any(Function), true);
      removed.mockRestore();
    });

    it("is Ctrl+B where Cmd is not the key, and leaves a plain b alone", async () => {
      pressInSidebar({ key: "b", ctrlKey: true });
      await flushPromises();
      expect(wrapper.findAllComponents({ name: "SplitterPanel" })[0]!.emitted("collapse")).toBeTruthy();

      pressInSidebar({ key: "b", metaKey: true });
      await flushPromises();
      expect(wrapper.findAllComponents({ name: "SplitterPanel" })[0]!.emitted("collapse")).toHaveLength(1);

      const plain = pressInSidebar({ key: "b" });
      expect(plain.defaultPrevented).toBe(false);
      await flushPromises();
      expect(wrapper.findAllComponents({ name: "SplitterPanel" })[0]!.emitted("collapse")).toHaveLength(1);
      wrapper.unmount();
    });
  });
  describe("the settings dialog", () => {
    const openSettings = async (settings = DEFAULT_SETTINGS) => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        {
          settings: cloneSettings(settings),
        },
      );
      await wrapper.get('[data-testid="settings-button"]').trigger("click");
      return wrapper;
    };

    it("keeps settings hidden and zoom inert until the saved preferences load", async () => {
      let resolveSettings!: (settings: AppSettings) => void;
      const settingsLoad = new Promise<AppSettings>((resolve) => (resolveSettings = resolve));
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { settingsLoad },
      );

      expect(wrapper.find('[data-testid="settings-button"]').exists()).toBe(false);
      const zoom = new KeyboardEvent("keydown", { key: "+", metaKey: true, cancelable: true });
      window.dispatchEvent(zoom);
      expect(zoom.defaultPrevented).toBe(false);
      expect(mocks.saveSettings).not.toHaveBeenCalled();

      resolveSettings({
        ...cloneSettings(DEFAULT_SETTINGS),
        ui: { fontSize: 18, zoom: 1, theme: "system" as const },
      });
      await flushPromises();
      expect(wrapper.find('[data-testid="settings-button"]').exists()).toBe(true);
      expect(document.documentElement.style.getPropertyValue("--marvis-ui-font-scale")).toBe(String(18 / 14));
      wrapper.unmount();
    });

    it("opens off the gear, draws a control for every preference, and closes on Cancel", async () => {
      const wrapper = await openSettings();
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

      // Every field in the schema has a control, and no control is drawn for anything else: the
      // form is the schema, and the test reads the schema rather than a copy of it.
      for (const section of SETTINGS_SECTIONS) {
        for (const field of section.fields) {
          const control = `settings-${field.path.replaceAll(".", "-")}`;
          // Inputs are labelled by `for`/`id` and pickers by the testid their trigger carries, so
          // the check asks for either rather than making the test know which kind of field it is.
          expect(wrapper.find(`#${control}, [data-testid="${control}"]`).exists(), field.path).toBe(true);
        }
      }

      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Cancel")!
        .trigger("click");
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(mocks.saveSettings).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("draws every credit the About section carries, each with the licence it travels under", async () => {
      const wrapper = await openSettings();
      // Read the list rather than a copy of it, for the reason the field test above does: what
      // matters is that nothing in `credits.ts` is dropped on the way to the screen.
      const about = wrapper.get('[data-testid="about-section"]').text();
      expect(about).toContain(ACKNOWLEDGEMENT);
      for (const group of CREDITS) {
        expect(about, group.title).toContain(group.title);
        for (const entry of group.entries) {
          expect(about, entry.name).toContain(entry.name);
          expect(about, entry.name).toContain(entry.license);
        }
      }
      // And the repository, which is the one thing a person in this section is most likely to want.
      expect(about).toContain(REPOSITORY);
      wrapper.unmount();
    });

    it("paints the window in the theme the preference names, and in the system's while it does not", async () => {
      const listeners: Array<() => void> = [];
      const system = {
        matches: true,
        addEventListener: (_: string, listener: () => void) => listeners.push(listener),
        removeEventListener: vi.fn(),
      };
      vi.spyOn(window, "matchMedia").mockReturnValue(system as unknown as MediaQueryList);

      // The attribute is the whole of it: the two palettes in `marvis.css` are keyed off it, and a
      // palette the preference names outright is followed rather than argued with.
      const light = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        {
          settings: { ...cloneSettings(DEFAULT_SETTINGS), ui: { ...DEFAULT_SETTINGS.ui, theme: "light" } },
        },
      );
      expect(document.documentElement.dataset.theme).toBe("light");
      // A window told what to draw in does not consult the system, but it still listens for it:
      // the listener is registered once for the shell's lifetime, not per preference change.
      system.matches = false;
      for (const listener of listeners) listener();
      expect(document.documentElement.dataset.theme).toBe("light");
      light.unmount();
      expect(system.removeEventListener).toHaveBeenCalledWith("change", expect.any(Function));

      system.matches = true;
      const following = await mountApp(workspaceWith(checkout("checkout:one")));
      expect(document.documentElement.dataset.theme).toBe("dark");

      // `system` is the preference that says to keep asking: the answer changes while the window is
      // open, and a window that was asked once would sit in the palette it started in.
      system.matches = false;
      for (const listener of listeners) listener();
      expect(document.documentElement.dataset.theme).toBe("light");
      following.unmount();
    });

    it("moves focus into the dialog, traps Tab, and restores the gear on close", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { attachTo: document.body },
      );
      const gear = wrapper.get('[data-testid="settings-button"]').element;
      await wrapper.get('[data-testid="settings-button"]').trigger("click");
      await flushPromises();

      expect(document.activeElement).toBe(wrapper.get("button[autofocus]").element);
      const apply = wrapper.findAll("button").find((button) => button.text() === "Apply")!.element;
      apply.focus();
      apply.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(wrapper.get('button[aria-label="Close"]').element);

      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Cancel")!
        .trigger("click");
      await flushPromises();
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(document.activeElement).toBe(gear);
      wrapper.unmount();
    });

    it("locks the draft while saving, ignores zoom, and retains it after a failed save", async () => {
      let rejectSave!: (error: Error) => void;
      mocks.saveSettings.mockImplementationOnce(() => new Promise<void>((_, reject) => (rejectSave = reject)));
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        { attachTo: document.body },
      );
      await wrapper.get('[data-testid="settings-button"]').trigger("click");
      const fontSize = wrapper.get("#settings-editor-fontSize");
      await fontSize.setValue("15");
      await fontSize.trigger("change");
      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Apply")!
        .trigger("click");
      await flushPromises();

      expect(fontSize.attributes("disabled")).toBeDefined();
      expect(wrapper.get("button[aria-label=Close]").attributes("disabled")).toBeDefined();
      expect(
        wrapper
          .findAll("button")
          .find((button) => button.text() === "Cancel")!
          .attributes("disabled"),
      ).toBeDefined();
      expect(
        wrapper
          .findAll("button")
          .find((button) => button.text() === "Reset to defaults")!
          .attributes("disabled"),
      ).toBeDefined();
      expect(wrapper.find('[role="dialog"]').attributes("aria-busy")).toBe("true");
      const zoom = new KeyboardEvent("keydown", { key: "+", metaKey: true, cancelable: true });
      window.dispatchEvent(zoom);
      expect(mocks.saveSettings).toHaveBeenCalledOnce();
      expect(document.documentElement.style.getPropertyValue("zoom")).toBe("1");
      await wrapper.get('[role="dialog"]').trigger("keydown", { key: "Escape" });
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

      rejectSave(new Error("settings write failed"));
      await flushPromises();
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
      expect((wrapper.get("#settings-editor-fontSize").element as HTMLInputElement).value).toBe("15");
      expect(wrapper.get("#settings-editor-fontSize").attributes("disabled")).toBeUndefined();
      wrapper.unmount();
    });

    it("writes the whole set and takes effect at once, without waiting for the file", async () => {
      const wrapper = await openSettings();
      const fontSize = wrapper.get("#settings-ui-fontSize");
      await fontSize.setValue("18");
      await fontSize.trigger("change");

      // The draft is the dialog's own until Apply, so nothing on screen has moved yet.
      expect(document.documentElement.style.getPropertyValue("--marvis-ui-font-scale")).toBe("1");

      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Apply")!
        .trigger("click");
      await flushPromises();
      // Applied, the whole type scale on the document moves: the window is redrawn at 18/14 of
      // the size it was drawn at, with no reload and without waiting for the file to be written.
      expect(document.documentElement.style.getPropertyValue("--marvis-ui-font-scale")).toBe(String(18 / 14));
      expect(mocks.saveSettings).toHaveBeenCalledWith({
        ...DEFAULT_SETTINGS,
        ui: { ...DEFAULT_SETTINGS.ui, fontSize: 18 },
      });
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("says nothing has been written when Cancel throws the draft away", async () => {
      const wrapper = await openSettings();
      const fontSize = wrapper.get("#settings-terminal-fontSize");
      await fontSize.setValue("20");
      await fontSize.trigger("change");
      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Cancel")!
        .trigger("click");
      await wrapper.get('[data-testid="settings-button"]').trigger("click");

      // Reopening starts from what is saved, not from what was typed and abandoned.
      expect((wrapper.get("#settings-terminal-fontSize").element as HTMLInputElement).value).toBe("16");
      expect(mocks.saveSettings).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("rejects fractional indentation sizes instead of sending invalid settings", async () => {
      const wrapper = await openSettings();
      const size = wrapper.get("#settings-editor-indentation-size");
      await size.setValue("2.5");
      await size.trigger("change");

      expect((size.element as HTMLInputElement).value).toBe("2");
      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Apply")!
        .trigger("click");
      await flushPromises();
      expect(mocks.saveSettings).toHaveBeenCalledWith(DEFAULT_SETTINGS);
      wrapper.unmount();
    });

    it("stays open and says so when the write fails", async () => {
      mocks.saveSettings.mockRejectedValue(new Error("could not write the settings file"));
      const wrapper = await openSettings();
      const fontSize = wrapper.get("#settings-editor-fontSize");
      await fontSize.setValue("15");
      await fontSize.trigger("change");

      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Apply")!
        .trigger("click");
      await flushPromises();

      // Closing on a failed write would leave the window showing a setting the app is not using
      // and the file not carrying, with nothing on screen to say so.
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
      const { toasts } = useToasts();
      expect(toasts.value.at(-1)?.message).toContain("could not write the settings file");

      // And the window is back where the file says it should be, so reopening shows the saved
      // value rather than the one the file refused.
      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Cancel")!
        .trigger("click");
      await wrapper.get('[data-testid="settings-button"]').trigger("click");
      expect((wrapper.get("#settings-editor-fontSize").element as HTMLInputElement).value).toBe("13");
      wrapper.unmount();
    });

    it("puts the window back to the last saved set when a later write is refused", async () => {
      const wrapper = await mountApp(
        workspaceWith(checkout("checkout:one")),
        { ...DEFAULT_APP_LAYOUT },
        {
          settings: {
            ...cloneSettings(DEFAULT_SETTINGS),
            ui: { fontSize: 16, zoom: 1, theme: "system" as const },
          },
        },
      );
      mocks.saveSettings.mockRejectedValueOnce(new Error("the settings file could not be read"));

      window.dispatchEvent(new KeyboardEvent("keydown", { key: "+", metaKey: true, cancelable: true }));
      await flushPromises();

      // The scale is adopted first so a held key walks the steps, and taken back when the write
      // does not land: what is drawn and what the file carries are the same thing again.
      expect(document.documentElement.style.getPropertyValue("zoom")).toBe("1");
      expect(wrapper.findComponent({ name: "MainPane" }).props("zoom")).toBe(1);
      const { toasts } = useToasts();
      expect(toasts.value.at(-1)?.message).toContain("could not be read");
      wrapper.unmount();
    });

    it("puts the draft back to the defaults and applies that", async () => {
      const wrapper = await openSettings();
      const toggle = wrapper.get("#settings-terminal-ligatures");
      await toggle.setValue(false);
      expect((toggle.element as HTMLInputElement).checked).toBe(false);

      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Reset to defaults")!
        .trigger("click");
      expect((wrapper.get("#settings-terminal-ligatures").element as HTMLInputElement).checked).toBe(true);
      // Reset says where the draft would go; it is the same Apply that writes it.
      await wrapper
        .findAll("button")
        .find((button) => button.text() === "Apply")!
        .trigger("click");
      await flushPromises();
      expect(mocks.saveSettings).toHaveBeenCalledWith(DEFAULT_SETTINGS);
      wrapper.unmount();
    });

    it("hands the terminal and the editor the settings that were saved", async () => {
      const wrapper = await openSettings({
        ...DEFAULT_SETTINGS,
        terminal: { ...DEFAULT_SETTINGS.terminal, fontSize: 20, ligatures: false, scrollbar: "always" },
        editor: {
          ...DEFAULT_SETTINGS.editor,
          fontSize: 15,
          indentation: { useSpaces: false, size: 4 },
        },
      });

      const main = wrapper.findComponent({ name: "MainPane" });
      expect(main.props("terminalSettings")).toMatchObject({ fontSize: 20, ligatures: false, scrollbar: "always" });
      expect(main.props("editorSettings")).toMatchObject({ fontSize: 15 });
      wrapper.unmount();
    });
  });
});
