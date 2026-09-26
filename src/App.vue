<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { SplitterGroup, SplitterPanel, SplitterResizeHandle } from "reka-ui";
import type { Checkout } from "./domain/workspace";
import type { PaletteCommandId } from "./domain/command-palette";
import { getPaletteCommands } from "./domain/command-palette";
import { parseEditorPosition } from "./domain/editor";
import type { EditorPosition } from "./domain/editor";
import { resolveMainView } from "./domain/main-document";
import type { MainDocument, MainDocumentMode } from "./domain/main-document";
import InspectorPane from "./components/InspectorPane.vue";
import CommandPalette from "./components/CommandPalette.vue";
import DocumentPane from "./components/DocumentPane.vue";
import SessionPane from "./components/SessionPane.vue";
import Sidebar from "./components/Sidebar.vue";
import WorktreeDialog from "./components/WorktreeDialog.vue";
import type { TerminalSessionStatus, WorkspaceState } from "./domain/workspace";
import { isIpcError } from "./domain/ipc";
import {
  closeMissingCheckout,
  getEditorAvailability,
  locateMissingCheckout,
  openInZed,
  selectCheckout as persistCheckoutSelection,
} from "./lib/ipc";
import type { EditorAvailability } from "./lib/ipc";
import { useWorkspaceState } from "./presentation/workspace";
import { useActiveGitSnapshot } from "./presentation/active-git-snapshot";
import { useReviewNotes } from "./presentation/review-notes";
import { useAgentSessions } from "./presentation/agent-sessions";
import { defaultAgentSession } from "./domain/agent";
import { buildReviewMarkdown } from "./domain/review";
import { isMarkdownPath } from "./presentation/markdown-preview";
import {
  DEFAULT_APP_LAYOUT,
  DEFAULT_CHECKOUT_UI_STATE,
  needsInspectorDrawer,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  resizeLayoutPanel,
  toggleFocusLayout,
  toggleLayoutVisibility as toggleLayoutVisibilityState,
} from "./domain/ui-state";
import type { AppLayoutState, CheckoutUiState } from "./domain/ui-state";
import { loadAppLayout, loadCheckoutUiState, saveAppLayout, saveCheckoutUiState } from "./lib/ipc";

const {
  workspace,
  activeCheckout,
  isOpening,
  error,
  chooseFolder,
  selectCheckout,
  selectSession: selectWorkspaceSession,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const appLayout = ref<AppLayoutState>({ ...DEFAULT_APP_LAYOUT });
const appLayoutReady = ref(false);
const checkoutUiStates = ref<Record<string, CheckoutUiState>>({});
const checkoutUiReady = ref(false);
const sessionPane = ref<InstanceType<typeof SessionPane> | null>(null);
const sidebarPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const inspectorPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const paletteRequestToken = ref(0);
const viewportWidth = ref(window.innerWidth);
const isNarrow = computed(() => needsInspectorDrawer(appLayout.value, viewportWidth.value));
const activeRepo = computed(
  () =>
    workspace.value.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === activeCheckout.value?.id)) ??
    null,
);
const gitSnapshot = useActiveGitSnapshot(activeCheckout, activeRepo, () => void promptForDefaultBranchIfNeeded(true));
const review = useReviewNotes(activeCheckout, activeRepo);
const agent = useAgentSessions(activeCheckout, activeRepo);
const allCheckouts = computed(() => workspace.value.repos.flatMap((repo) => repo.checkouts));
const registeredSessionIds = computed(() =>
  workspace.value.repos.flatMap((repo) =>
    repo.checkouts.flatMap((checkout) => checkout.sessions.map((session) => session.id)),
  ),
);
const lifecycle = ref<{ mode: "create" | "remove"; checkoutId: string } | null>(null);

// A session that reappears after a crash may carry messages this client never saw, so any
// round left unconfirmed is settled as soon as the checkout's sessions can be read.
watch(
  () => agent.sessions.map((session) => `${session.id}:${session.updatedAt}`).join(","),
  (sessionsKey) => {
    if (sessionsKey) void review.reconcileRounds();
  },
);

// v2.0.18 has no turn-completed event, so a finished turn is observed rather than announced.
// When one is seen the round closes, which is what clears the "rounds not finished" count.
// What the turn did to each line is judged separately, by the diff.
watch(
  () => agent.turnsCompleted,
  async () => {
    // A queued round waited for exactly this: the agent is free now, so the reviews held
    // back while it was busy go out, oldest first.
    await review.flushQueuedRounds();
    await review.ackFinishedTurn();
  },
);
const shellRequest = ref<{ checkoutId: string; token: number } | null>(null);
const nvimRequest = ref<{
  checkoutId: string;
  filePath?: string;
  line?: number;
  column?: number;
  token: number;
} | null>(null);
const agentRequest = ref<{ checkoutId: string; prompt: string; token: number } | null>(null);
const sendingReview = ref(false);
const documents = ref<Record<string, MainDocument>>({});
const emptyDocument: MainDocument = { checkoutId: "", path: "", source: "file", mode: "code" };
const mainViews = ref<Record<string, "terminal" | "document">>({});
const inspectorCommand = ref<{
  action: "open-file" | "open-changes";
  token: number;
} | null>(null);
const editorAvailability = ref<EditorAvailability>({ zed: false, neovim: false });
const sessionRuntimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const recentFileWrites = ref<Record<string, boolean>>({});
const documentRefreshRevisions = ref<Record<string, number>>({});
const activityByCheckout = computed(() => {
  const activity: Record<string, string[]> = {};
  for (const checkout of allCheckouts.value) {
    const actors = checkout.sessions.flatMap((session) => {
      const status = sessionRuntimeStatuses.value[session.id];
      if (status?.state !== "running" || !status.foregroundProcess) return [];
      return [`${session.type === "nvim" ? "Neovim" : "Terminal"} · ${session.name}`];
    });
    if (recentFileWrites.value[checkout.id]) actors.push("Recent file writes");
    activity[checkout.id] = actors;
  }
  return activity;
});
let shellRequestToken = 0;
let nvimRequestToken = 0;
let inspectorCommandToken = 0;
let unlistenFileActivity: (() => void) | undefined;
let activityListenerDisposed = false;
let unlistenCloseRequested: (() => void) | undefined;
let allowWindowClose = false;
let windowClosePromise: Promise<void> | null = null;
let uiLayoutSaveTimer: number | undefined;
const checkoutUiSaveTimers = new Map<string, number>();
let uiStateWriteQueue: Promise<void> = Promise.resolve();
const loadedCheckoutUiIds = new Set<string>();
const pendingCheckoutUiPatches = new Map<string, Partial<CheckoutUiState>>();
let checkoutUiLoadGeneration = 0;
let userSplitterIntent = false;
let userSplitterKeyDown = false;
let synchronizingSplitters = false;
const activityExpiryTimers = new Map<string, number>();
const lifecycleCheckout = computed<Checkout | null>(
  () =>
    workspace.value.repos
      .flatMap((repo) => repo.checkouts)
      .find((checkout) => checkout.id === lifecycle.value?.checkoutId) ?? null,
);
const lifecycleRepo = computed(
  () =>
    workspace.value.repos.find((repo) =>
      repo.checkouts.some((checkout) => checkout.id === lifecycle.value?.checkoutId),
    ) ?? null,
);
const paletteCommands = computed(() => {
  const checkout = activeCheckout.value;
  const document = checkout ? documents.value[checkout.id] : undefined;
  return getPaletteCommands({
    hasCheckout: Boolean(checkout),
    isMissing: checkout?.isMissing ?? false,
    isGit: activeRepo.value?.kind === "git",
    hasSelectedFile: Boolean(document),
    zedAvailable: editorAvailability.value.zed,
    neovimAvailable: editorAvailability.value.neovim,
  });
});
const activeDocument = computed(() => {
  const checkout = activeCheckout.value;
  const document = checkout ? documents.value[checkout.id] : undefined;
  if (!checkout || !document || document.checkoutId !== checkout.id) return null;
  return document;
});
const documentPaneDocument = computed(() => activeDocument.value ?? emptyDocument);
const activeMainView = computed(() => {
  return resolveMainView(mainViews.value, activeCheckout.value?.id ?? null, activeDocument.value);
});

function openFileDocument(selection: { checkoutId: string; path: string }) {
  const previousDocument = documents.value[selection.checkoutId];
  documents.value = {
    ...documents.value,
    [selection.checkoutId]: {
      ...selection,
      source: "file",
      mode: isMarkdownPath(selection.path) ? "view" : "code",
    },
  };
  mainViews.value = { ...mainViews.value, [selection.checkoutId]: "document" };
  updateCheckoutUiState(selection.checkoutId, {
    document: documents.value[selection.checkoutId],
    mainView: "document",
    ...(previousDocument?.path !== selection.path && { documentScrollTop: 0, documentScrollLeft: 0 }),
  });
}

function openChangedDocument(selection: { checkoutId: string; path: string }) {
  const previousDocument = documents.value[selection.checkoutId];
  documents.value = { ...documents.value, [selection.checkoutId]: { ...selection, source: "change", mode: "diff" } };
  mainViews.value = { ...mainViews.value, [selection.checkoutId]: "document" };
  updateCheckoutUiState(selection.checkoutId, {
    document: documents.value[selection.checkoutId],
    mainView: "document",
    ...(previousDocument?.path !== selection.path && { documentScrollTop: 0, documentScrollLeft: 0 }),
  });
}

function setDocumentMode(mode: MainDocumentMode) {
  const checkoutId = activeCheckout.value?.id;
  const document = activeDocument.value;
  if (!checkoutId || !document) return;
  documents.value = { ...documents.value, [checkoutId]: { ...document, mode } };
  updateCheckoutUiState(checkoutId, { document: documents.value[checkoutId] });
}

function activateCheckoutTerminal(checkoutId: string) {
  // Selecting a checkout restores its saved main view. Only explicit terminal actions switch views.
  void selectCheckout(checkoutId);
}

async function activateTerminalSession(sessionId: string) {
  const checkout = allCheckouts.value.find((item) => item.sessions.some((session) => session.id === sessionId));
  if (!checkout) return;
  await selectWorkspaceSession(sessionId);
  mainViews.value = { ...mainViews.value, [checkout.id]: "terminal" };
  updateCheckoutUiState(checkout.id, { mainView: "terminal" });
  await nextTick();
  sessionPane.value?.focusActiveTerminal();
}

function updateCheckoutUiState(checkoutId: string, patch: Partial<CheckoutUiState>) {
  const previous = checkoutUiStates.value[checkoutId] ?? { ...DEFAULT_CHECKOUT_UI_STATE };
  const state = normalizeCheckoutUiState({ ...previous, ...patch, version: 1 });
  checkoutUiStates.value = { ...checkoutUiStates.value, [checkoutId]: state };
  if (!loadedCheckoutUiIds.has(checkoutId)) {
    pendingCheckoutUiPatches.set(checkoutId, { ...pendingCheckoutUiPatches.get(checkoutId), ...patch });
  }
  if (checkoutUiReady.value && loadedCheckoutUiIds.has(checkoutId)) scheduleCheckoutUiSave(checkoutId);
}

function updateDocumentReadingPosition(checkoutId: string, position: { top: number; left: number }) {
  updateCheckoutUiState(checkoutId, {
    documentScrollTop: Math.round(position.top),
    documentScrollLeft: Math.round(position.left),
  });
}

function updateDiffReadingPosition(checkoutId: string, top: number) {
  updateCheckoutUiState(checkoutId, { diffScrollTop: Math.round(top) });
}

function updateInspectorUiState(
  checkoutId: string,
  patch: Pick<
    CheckoutUiState,
    | "inspectorTab"
    | "selectedFilePath"
    | "selectedChangePath"
    | "expandedDirectories"
    | "filesScrollTop"
    | "changesScrollTop"
  >,
) {
  updateCheckoutUiState(checkoutId, patch);
}

function scheduleUiWrite(write: () => Promise<void>) {
  const pending = uiStateWriteQueue
    .catch(() => {})
    .then(write)
    .catch((cause: unknown) => {
      error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
    });
  uiStateWriteQueue = pending;
}

function scheduleAppLayoutSave() {
  if (!appLayoutReady.value) return;
  if (uiLayoutSaveTimer !== undefined) window.clearTimeout(uiLayoutSaveTimer);
  uiLayoutSaveTimer = window.setTimeout(() => {
    uiLayoutSaveTimer = undefined;
    const state = normalizeAppLayout(JSON.parse(JSON.stringify(appLayout.value)));
    scheduleUiWrite(() => saveAppLayout(state));
  }, 250);
}

function scheduleCheckoutUiSave(checkoutId: string) {
  const previousTimer = checkoutUiSaveTimers.get(checkoutId);
  if (previousTimer !== undefined) window.clearTimeout(previousTimer);
  checkoutUiSaveTimers.set(
    checkoutId,
    window.setTimeout(() => {
      checkoutUiSaveTimers.delete(checkoutId);
      const state = normalizeCheckoutUiState(JSON.parse(JSON.stringify(checkoutUiStates.value[checkoutId])));
      scheduleUiWrite(() => saveCheckoutUiState(checkoutId, state));
    }, 250),
  );
}

async function flushUiStateWrites() {
  if (uiLayoutSaveTimer !== undefined) {
    window.clearTimeout(uiLayoutSaveTimer);
    uiLayoutSaveTimer = undefined;
    const state = normalizeAppLayout(JSON.parse(JSON.stringify(appLayout.value)));
    scheduleUiWrite(() => saveAppLayout(state));
  }
  for (const [checkoutId, timer] of checkoutUiSaveTimers) {
    window.clearTimeout(timer);
    const state = normalizeCheckoutUiState(JSON.parse(JSON.stringify(checkoutUiStates.value[checkoutId])));
    scheduleUiWrite(() => saveCheckoutUiState(checkoutId, state));
  }
  checkoutUiSaveTimers.clear();
  await uiStateWriteQueue;
}

function toggleFocusMode() {
  appLayout.value = toggleFocusLayout(appLayout.value);
  flushAfterLayoutInteraction();
}

function toggleLayoutVisibility(key: "sidebarVisible" | "inspectorVisible") {
  appLayout.value = toggleLayoutVisibilityState(appLayout.value, key);
  flushAfterLayoutInteraction();
}

function toggleTransparency() {
  appLayout.value = { ...appLayout.value, reduceTransparency: !appLayout.value.reduceTransparency };
  flushAfterLayoutInteraction();
}

function flushAfterLayoutInteraction() {
  void nextTick(() => flushUiStateWrites());
}

function updateCollapsedRepos(repoIds: string[]) {
  appLayout.value = { ...appLayout.value, collapsedRepoIds: repoIds };
  flushAfterLayoutInteraction();
}

function onSplitterDragging(dragging: boolean) {
  userSplitterIntent = dragging || userSplitterKeyDown;
  if (!dragging && !userSplitterKeyDown) flushAfterLayoutInteraction();
}

function onSplitterKeydown(event: KeyboardEvent) {
  if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  userSplitterKeyDown = true;
  userSplitterIntent = true;
}

function onSplitterKeyup(event: KeyboardEvent) {
  if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  userSplitterKeyDown = false;
  userSplitterIntent = false;
  flushAfterLayoutInteraction();
}

function onPanelCollapse(panel: "sidebar" | "inspector") {
  if (synchronizingSplitters || !userSplitterIntent) return;
  const key = panel === "sidebar" ? "sidebarVisible" : "inspectorVisible";
  if (!appLayout.value[key]) return;
  appLayout.value = { ...appLayout.value, [key]: false };
}

function onPanelExpand(panel: "sidebar" | "inspector") {
  if (synchronizingSplitters || !userSplitterIntent) return;
  const key = panel === "sidebar" ? "sidebarVisible" : "inspectorVisible";
  if (appLayout.value[key]) return;
  appLayout.value = { ...appLayout.value, [key]: true };
}

function ensureInspectorVisible() {
  if (appLayout.value.focusSnapshot) {
    appLayout.value = {
      ...appLayout.value,
      ...appLayout.value.focusSnapshot,
      focusSnapshot: null,
      inspectorVisible: true,
    };
  } else if (!appLayout.value.inspectorVisible) {
    appLayout.value = { ...appLayout.value, inspectorVisible: true };
  }
}

function onSplitterLayout(sizes: number[]) {
  if (synchronizingSplitters || !appLayoutReady.value || !userSplitterIntent || sizes.length < 3) return;
  let next = appLayout.value;
  if (sizes[0] > 0) next = resizeLayoutPanel(next, "sidebar", sizes[0]);
  if (!isNarrow.value && sizes[2] > 0) next = resizeLayoutPanel(next, "inspector", sizes[2]);
  if (next.focusSnapshot) {
    next = {
      ...next,
      focusSnapshot: {
        ...next.focusSnapshot,
        sidebarWidth: next.sidebarWidth,
        inspectorWidth: next.inspectorWidth,
      },
    };
  }
  if (next.sidebarWidth !== appLayout.value.sidebarWidth || next.inspectorWidth !== appLayout.value.inspectorWidth) {
    appLayout.value = next;
  }
}

function resetPanelWidth(panel: "sidebar" | "inspector") {
  const width = panel === "sidebar" ? DEFAULT_APP_LAYOUT.sidebarWidth : DEFAULT_APP_LAYOUT.inspectorWidth;
  withSplitterSynchronization(() => (panel === "sidebar" ? sidebarPanel.value : inspectorPanel.value)?.resize(width));
  appLayout.value = resizeLayoutPanel(appLayout.value, panel, width);
  if (appLayout.value.focusSnapshot) {
    appLayout.value = {
      ...appLayout.value,
      focusSnapshot: { ...appLayout.value.focusSnapshot, [`${panel}Width`]: width },
    };
  }
  flushAfterLayoutInteraction();
}

function resetLayout() {
  appLayout.value = { ...DEFAULT_APP_LAYOUT };
  synchronizeSplitterPanels();
  flushAfterLayoutInteraction();
}

function withSplitterSynchronization(action: () => void) {
  const wasSynchronizing = synchronizingSplitters;
  synchronizingSplitters = true;
  try {
    action();
  } finally {
    synchronizingSplitters = wasSynchronizing;
  }
}

function synchronizeSplitterPanels() {
  withSplitterSynchronization(() => {
    if (appLayout.value.sidebarVisible) {
      sidebarPanel.value?.expand();
      sidebarPanel.value?.resize(appLayout.value.sidebarWidth);
    } else sidebarPanel.value?.collapse();
    if (appLayout.value.inspectorVisible && !isNarrow.value) {
      inspectorPanel.value?.expand();
      inspectorPanel.value?.resize(appLayout.value.inspectorWidth);
    } else inspectorPanel.value?.collapse();
  });
}

function setMainView(view: "terminal" | "document") {
  const checkoutId = activeCheckout.value?.id;
  if (!checkoutId) return;
  mainViews.value = { ...mainViews.value, [checkoutId]: view };
  updateCheckoutUiState(checkoutId, { mainView: view });
  if (view === "terminal") void nextTick(() => sessionPane.value?.focusActiveTerminal());
}

watch(appLayout, () => scheduleAppLayoutSave(), { deep: true });

watch(
  [() => appLayout.value.sidebarVisible, () => appLayout.value.inspectorVisible, isNarrow, appLayoutReady],
  async () => {
    if (!appLayoutReady.value) return;
    await nextTick();
    synchronizeSplitterPanels();
  },
  { immediate: true, flush: "post" },
);

watch(
  () => activeCheckout.value?.id,
  async (checkoutId) => {
    const request = ++checkoutUiLoadGeneration;
    checkoutUiReady.value = false;
    if (!checkoutId) {
      checkoutUiReady.value = true;
      return;
    }
    if (loadedCheckoutUiIds.has(checkoutId)) {
      checkoutUiReady.value = true;
      return;
    }
    let state = { ...DEFAULT_CHECKOUT_UI_STATE };
    try {
      state = normalizeCheckoutUiState(await loadCheckoutUiState(checkoutId));
    } catch (cause) {
      if (request === checkoutUiLoadGeneration) {
        showWindowError(cause);
      }
    } finally {
      if (request === checkoutUiLoadGeneration) {
        state = normalizeCheckoutUiState({ ...state, ...pendingCheckoutUiPatches.get(checkoutId) });
        pendingCheckoutUiPatches.delete(checkoutId);
        checkoutUiStates.value = { ...checkoutUiStates.value, [checkoutId]: state };
        if (state.document?.checkoutId === checkoutId)
          documents.value = { ...documents.value, [checkoutId]: state.document };
        mainViews.value = {
          ...mainViews.value,
          [checkoutId]:
            state.mainView === "document" && state.document?.checkoutId === checkoutId ? "document" : "terminal",
        };
        loadedCheckoutUiIds.add(checkoutId);
        checkoutUiReady.value = true;
      }
    }
  },
  { immediate: true, flush: "sync" },
);

onMounted(async () => {
  window.addEventListener("resize", onViewportResize);
  window.addEventListener("keydown", onWindowKeydown);
  const currentWindow = getCurrentWindow();
  try {
    unlistenCloseRequested = await currentWindow.onCloseRequested(async (event) => {
      if (allowWindowClose) return;
      event.preventDefault();
      await requestWindowClose(currentWindow);
    });
  } catch (cause) {
    showWindowError(cause);
  }
  try {
    appLayout.value = normalizeAppLayout(await loadAppLayout());
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
    appLayout.value = { ...DEFAULT_APP_LAYOUT };
  } finally {
    appLayoutReady.value = true;
  }
  try {
    const dispose = await listen<string>("checkout-file-activity", (event) => {
      recentFileWrites.value = { ...recentFileWrites.value, [event.payload]: true };
      documentRefreshRevisions.value = {
        ...documentRefreshRevisions.value,
        [event.payload]: (documentRefreshRevisions.value[event.payload] ?? 0) + 1,
      };
      const previous = activityExpiryTimers.get(event.payload);
      if (previous !== undefined) window.clearTimeout(previous);
      activityExpiryTimers.set(
        event.payload,
        window.setTimeout(() => {
          const remaining = { ...recentFileWrites.value };
          delete remaining[event.payload];
          recentFileWrites.value = remaining;
          activityExpiryTimers.delete(event.payload);
        }, 5000),
      );
    });
    if (activityListenerDisposed) dispose();
    else unlistenFileActivity = dispose;
  } catch {
    // File-write activity is optional; PTY foreground-process activity remains observable.
  }
  try {
    editorAvailability.value = await getEditorAvailability();
  } catch {
    editorAvailability.value = { zed: false, neovim: false };
  }
});

onUnmounted(() => {
  activityListenerDisposed = true;
  unlistenCloseRequested?.();
  window.removeEventListener("resize", onViewportResize);
  window.removeEventListener("keydown", onWindowKeydown);
  unlistenFileActivity?.();
  for (const timer of activityExpiryTimers.values()) window.clearTimeout(timer);
  activityExpiryTimers.clear();
  if (uiLayoutSaveTimer !== undefined) window.clearTimeout(uiLayoutSaveTimer);
  for (const timer of checkoutUiSaveTimers.values()) window.clearTimeout(timer);
  checkoutUiSaveTimers.clear();
});

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
}

function showWindowError(cause: unknown) {
  error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}

function requestWindowClose(currentWindow: ReturnType<typeof getCurrentWindow>): Promise<void> {
  if (allowWindowClose) return Promise.resolve();
  if (windowClosePromise) return windowClosePromise;
  windowClosePromise = (async () => {
    await flushUiStateWrites();
    allowWindowClose = true;
    try {
      await currentWindow.close();
    } catch (cause) {
      allowWindowClose = false;
      showWindowError(cause);
    } finally {
      windowClosePromise = null;
    }
  })();
  return windowClosePromise;
}

function onWindowKeydown(event: KeyboardEvent) {
  if (!(event.metaKey || event.ctrlKey)) return;
  if (event.key.toLowerCase() === "q") {
    event.preventDefault();
    void requestWindowClose(getCurrentWindow());
    return;
  }
  if (isLayoutShortcutTarget(event.target)) return;
  if (event.key === "0" && !event.altKey && !event.shiftKey) {
    event.preventDefault();
    toggleLayoutVisibility("sidebarVisible");
  } else if (event.key === "0" && event.altKey && !event.shiftKey) {
    event.preventDefault();
    toggleLayoutVisibility("inspectorVisible");
  }
}

function isLayoutShortcutTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLSelectElement ||
    target instanceof HTMLTextAreaElement ||
    target.isContentEditable ||
    Boolean(target.closest(".xterm"))
  );
}

function openWorktreeDialog(mode: "create" | "remove", checkoutId: string) {
  lifecycle.value = { mode, checkoutId };
}

async function requestShell(checkoutId: string) {
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
  updateCheckoutUiState(checkoutId, { mainView: "terminal" });
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
    shellRequest.value = { checkoutId, token: ++shellRequestToken };
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}

async function requestNvim(checkoutId: string, filePath?: string, position?: EditorPosition) {
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
  updateCheckoutUiState(checkoutId, { mainView: "terminal" });
  const request = {
    checkoutId,
    ...(filePath && position && { filePath, line: position.line, column: position.column }),
    token: ++nvimRequestToken,
  };
  nvimRequest.value = request;
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
  } catch (cause) {
    if (nvimRequest.value?.token === request.token) nvimRequest.value = null;
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}

/**
 * Ships the notes the caller chose to one agent session as a single message.
 *
 * The round is recorded before the agent is called, so an interrupted send is recognizable
 * afterwards instead of being repeated. The target comes from the bridge, so a review can
 * never land in a session belonging to another checkout.
 */
async function sendReviewToAgent(ids: string[], queue = false) {
  const checkout = activeCheckout.value;
  // Read from the payload rather than the list: the sender decides what is included.
  const chosen = new Set(ids);
  const notes = review.notes.filter((note) => chosen.has(note.id));
  if (!checkout || checkout.isMissing || activeRepo.value?.kind !== "git" || notes.length === 0) return;
  if (sendingReview.value) return;
  const status = gitSnapshot.status;
  const markdown = buildReviewMarkdown(notes, {
    branch: status?.branch,
    defaultBranch: status?.defaultBranch,
  });
  sendingReview.value = true;
  try {
    const target = await resolveAgentTarget();
    if (!target) return;
    // `queue` records the round and leaves the agent alone: the message it was accepted
    // with is stored with it, so the flush that runs at the end of the turn sends exactly
    // those bytes, even after a restart.
    await review.dispatchRound(
      target,
      notes.map((note) => note.id),
      markdown,
      queue,
    );
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  } finally {
    sendingReview.value = false;
  }
}

/** The session a review goes to: the selected one, otherwise the newest live session. */
async function resolveAgentTarget(): Promise<string | null> {
  const target = agent.targetId ?? defaultAgentSession(agent.sessions)?.id ?? null;
  if (target) return target;
  // No session yet: start one so the review is not silently dropped.
  const created = await agent.createSession(`Review ${new Date().toISOString().slice(0, 10)}`);
  return created?.id ?? null;
}

function promptEditorPosition(): EditorPosition | null {
  const value = window.prompt("Open selected file at line[:column]:", "1");
  if (value === null) return null;
  const position = parseEditorPosition(value);
  if (!position) error.value = "Enter a positive line number with an optional column (for example, 42:7).";
  else error.value = null;
  return position;
}

function requestInspector(action: "open-file" | "open-changes") {
  inspectorCommand.value = { action, token: ++inspectorCommandToken };
}

function updateSessionStatus(sessionId: string, status: TerminalSessionStatus | null) {
  if (status) sessionRuntimeStatuses.value[sessionId] = status;
  else delete sessionRuntimeStatuses.value[sessionId];
}

async function closeTerminalSession(sessionId: string) {
  await sessionPane.value?.requestClose(sessionId);
}

async function runPaletteCommand(command: PaletteCommandId) {
  const checkout = activeCheckout.value;
  switch (command) {
    case "open-directory":
      await chooseFolder();
      break;
    case "toggle-focus":
      toggleFocusMode();
      break;
    case "toggle-sidebar":
      toggleLayoutVisibility("sidebarVisible");
      break;
    case "toggle-inspector":
      toggleLayoutVisibility("inspectorVisible");
      break;
    case "toggle-transparency":
      toggleTransparency();
      break;
    case "reset-layout":
      resetLayout();
      break;
    case "new-worktree":
      if (activeRepo.value?.kind === "git") {
        const primary = activeRepo.value.checkouts.find((item) => item.isPrimary && !item.isMissing);
        if (primary) openWorktreeDialog("create", primary.id);
      }
      break;
    case "new-terminal":
      if (checkout && !checkout.isMissing) await requestShell(checkout.id);
      break;
    case "open-file":
      if (checkout && !checkout.isMissing) {
        ensureInspectorVisible();
        requestInspector("open-file");
      }
      break;
    case "open-changes":
      if (checkout && activeRepo.value?.kind === "git" && !checkout.isMissing) {
        ensureInspectorVisible();
        requestInspector("open-changes");
      }
      break;
    case "open-preview":
      if (activeDocument.value) {
        const mode = isMarkdownPath(activeDocument.value.path) ? "view" : "code";
        setDocumentMode(mode);
        setMainView("document");
      }
      break;
    case "open-zed":
      if (checkout && !checkout.isMissing) {
        const file = activeDocument.value?.checkoutId === checkout.id ? activeDocument.value.path : undefined;
        const position = file ? promptEditorPosition() : undefined;
        if (file && !position) break;
        try {
          await openInZed(checkout.id, file, position?.line, position?.column);
        } catch (cause) {
          error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
        }
      }
      break;
    case "open-neovim":
      if (checkout && !checkout.isMissing) {
        const file = activeDocument.value?.checkoutId === checkout.id ? activeDocument.value.path : undefined;
        const position = file ? promptEditorPosition() : undefined;
        if (file && !position) break;
        await requestNvim(checkout.id, file, position ?? undefined);
      }
      break;
  }
}

function applyWorkspace(next: WorkspaceState) {
  updateWorkspace(next);
}

function reportWarning(message: string) {
  error.value = message;
}

async function locateCheckout(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  if (!checkout) return;
  error.value = null;
  isOpening.value = true;
  try {
    const parent = checkout.path.replace(/[\\/][^\\/]*$/, "") || "/";
    const path = await open({
      directory: true,
      multiple: false,
      title: "Locate missing checkout",
      defaultPath: parent,
    });
    if (typeof path === "string") workspace.value = await locateMissingCheckout(checkoutId, path);
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  } finally {
    isOpening.value = false;
  }
}

async function closeCheckout(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  const repo = workspace.value.repos.find((item) => item.checkouts.some((entry) => entry.id === checkoutId));
  if (!checkout || !repo) return;
  const closesRepo = checkout.isPrimary || repo.kind === "plain";
  const scope = closesRepo ? `“${repo.name}” and its checkout list` : `“${checkout.path}”`;
  if (!window.confirm(`Close ${scope} in Marvis? No files will be deleted.`)) return;
  try {
    workspace.value = await closeMissingCheckout(checkoutId);
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}
</script>

<template>
  <div
    v-if="appLayoutReady"
    class="app-shell relative flex h-full min-w-[900px] flex-col text-zinc-100"
    :class="{ 'reduce-transparency': appLayout.reduceTransparency }"
    :style="{ '--inspector-width': `${appLayout.inspectorWidth}px` }"
  >
    <header class="window-header flex h-12 shrink-0 items-center border-b text-zinc-300">
      <div class="flex h-full shrink-0 items-center pl-[82px] pr-3">
        <button
          type="button"
          data-testid="command-field"
          aria-label="Search files and commands"
          class="window-search flex h-8 w-[min(300px,34vw)] items-center gap-2 rounded-md border px-2.5 text-left text-xs text-zinc-400 hover:text-zinc-200"
          @click="paletteRequestToken += 1"
        >
          <span aria-hidden="true" class="text-sm">⌕</span>
          <span class="min-w-0 flex-1 truncate">Search files and commands…</span>
          <kbd class="shrink-0 rounded border border-white/10 px-1 py-0.5 font-sans text-[10px] text-zinc-500">⌘K</kbd>
        </button>
      </div>
      <!-- Keep native dragging on this empty spacer only, clear of controls and visual effects. -->
      <div data-tauri-drag-region aria-hidden="true" class="h-full min-w-4 flex-1" />
      <nav
        aria-label="Repository location"
        class="window-breadcrumb flex h-full min-w-0 max-w-[42%] shrink-0 items-center gap-2 pr-4 text-xs"
      >
        <button
          v-if="activeCheckout"
          type="button"
          title="Toggle navigation sidebar"
          class="max-w-40 truncate text-zinc-300 hover:text-white"
          @click="toggleLayoutVisibility('sidebarVisible')"
        >
          {{ activeRepo?.name ?? activeCheckout.path.split(/[\\/]/).at(-1) }}
        </button>
        <template v-if="activeCheckout && activeRepo?.kind === 'git'">
          <span aria-hidden="true" class="text-zinc-600">/</span>
          <span class="max-w-32 truncate text-zinc-500">{{ activeCheckout.branch || "Detached" }}</span>
        </template>
        <template v-if="activeCheckout">
          <span aria-hidden="true" class="text-zinc-600">/</span>
          <template v-if="activeDocument">
            <button
              v-if="activeMainView === 'terminal'"
              type="button"
              data-testid="document-breadcrumb"
              class="max-w-48 truncate text-zinc-500 hover:text-zinc-200"
              :title="activeDocument.path"
              @click="setMainView('document')"
            >
              {{ activeDocument.path.split(/[\\/]/).at(-1) }}
            </button>
            <span v-else class="max-w-48 truncate text-zinc-200" :title="activeDocument.path">
              {{ activeDocument.path.split(/[\\/]/).at(-1) }}
            </span>
            <span aria-hidden="true" class="text-zinc-600">/</span>
            <button
              v-if="activeMainView === 'document'"
              type="button"
              data-testid="terminal-breadcrumb"
              class="text-zinc-500 hover:text-zinc-200"
              @click="setMainView('terminal')"
            >
              Terminal
            </button>
            <span v-else class="text-zinc-200">Terminal</span>
          </template>
          <span v-else class="text-zinc-200">Terminal</span>
        </template>
      </nav>
    </header>
    <SplitterGroup direction="horizontal" class="app-splitter flex min-h-0 flex-1" @layout="onSplitterLayout">
      <SplitterPanel
        id="navigation-panel"
        ref="sidebarPanel"
        :default-size="appLayout.sidebarVisible ? appLayout.sidebarWidth : 0"
        :min-size="220"
        :max-size="380"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="min-h-0 shrink-0"
        @collapse="onPanelCollapse('sidebar')"
        @expand="onPanelExpand('sidebar')"
      >
        <Sidebar
          v-show="appLayout.sidebarVisible"
          :repos="workspace.repos"
          :active-checkout-id="workspace.activeCheckoutId"
          :active-session-id="workspace.activeSessionId"
          :collapsed-repo-ids="appLayout.collapsedRepoIds"
          :activity-by-checkout="activityByCheckout"
          :session-runtime-statuses="sessionRuntimeStatuses"
          :is-opening="isOpening"
          @open-folder="chooseFolder"
          @select-checkout="activateCheckoutTerminal"
          @select-session="activateTerminalSession"
          @locate-missing="locateCheckout"
          @close-missing="closeCheckout"
          @create-worktree="openWorktreeDialog('create', $event)"
          @new-terminal="requestShell"
          @remove-worktree="openWorktreeDialog('remove', $event)"
          @close-session="closeTerminalSession"
          @update-collapsed-repos="updateCollapsedRepos"
        />
      </SplitterPanel>
      <SplitterResizeHandle
        id="navigation-resize-handle"
        aria-label="Resize navigation sidebar"
        class="splitter-handle"
        @dragging="onSplitterDragging"
        @keydown.capture="onSplitterKeydown"
        @keyup.capture="onSplitterKeyup"
        @dblclick.stop="resetPanelWidth('sidebar')"
      />
      <SplitterPanel id="main-panel" :min-size="420" size-unit="px" class="main-column min-h-0 min-w-0 flex-1">
        <div class="relative min-h-0 flex-1" :aria-busy="!checkoutUiReady">
          <section
            v-show="activeMainView === 'terminal'"
            id="main-view-terminal"
            key="terminal"
            class="absolute inset-0"
          >
            <SessionPane
              ref="sessionPane"
              class="absolute inset-0"
              :checkout="checkoutUiReady ? activeCheckout : null"
              :active-session-id="workspace.activeSessionId"
              :is-opening="isOpening || !checkoutUiReady"
              :visible="checkoutUiReady && activeMainView === 'terminal'"
              :shell-request="shellRequest"
              :nvim-request="nvimRequest"
              :agent-request="agentRequest"
              :registered-session-ids="registeredSessionIds"
              @open-folder="chooseFolder"
              @workspace-updated="updateWorkspace"
              @session-status-changed="updateSessionStatus"
            />
          </section>
          <section
            v-show="activeMainView === 'document' && activeDocument && checkoutUiReady"
            id="main-view-document"
            key="document"
            class="absolute inset-0"
          >
            <DocumentPane
              :checkout="activeCheckout"
              :document="documentPaneDocument"
              :git-snapshot="gitSnapshot"
              :review="review"
              :active="activeMainView === 'document'"
              :refresh-revision="documentRefreshRevisions[documentPaneDocument.checkoutId] ?? 0"
              :zed-available="editorAvailability.zed"
              :reading-position="{
                top: checkoutUiStates[documentPaneDocument.checkoutId]?.documentScrollTop ?? 0,
                left: checkoutUiStates[documentPaneDocument.checkoutId]?.documentScrollLeft ?? 0,
              }"
              :diff-scroll-top="checkoutUiStates[documentPaneDocument.checkoutId]?.diffScrollTop ?? 0"
              @update-mode="setDocumentMode"
              @reading-position-changed="updateDocumentReadingPosition(documentPaneDocument.checkoutId, $event)"
              @diff-position-changed="updateDiffReadingPosition(documentPaneDocument.checkoutId, $event)"
              @open-markdown-link="openFileDocument({ checkoutId: documentPaneDocument.checkoutId, path: $event })"
              @open-in-zed="runPaletteCommand('open-zed')"
            />
          </section>
        </div>
      </SplitterPanel>
      <SplitterResizeHandle
        id="inspector-resize-handle"
        aria-label="Resize files and changes inspector"
        class="splitter-handle"
        @dragging="onSplitterDragging"
        @keydown.capture="onSplitterKeydown"
        @keyup.capture="onSplitterKeyup"
        @dblclick.stop="resetPanelWidth('inspector')"
      />
      <SplitterPanel
        id="inspector-panel"
        ref="inspectorPanel"
        :default-size="appLayout.inspectorVisible && !isNarrow ? appLayout.inspectorWidth : 0"
        :min-size="260"
        :max-size="560"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="inspector-splitter-panel relative min-h-0 shrink-0 overflow-visible"
        @collapse="onPanelCollapse('inspector')"
        @expand="onPanelExpand('inspector')"
      >
        <InspectorPane
          v-show="appLayout.inspectorVisible"
          :class="{ 'right-inspector-drawer': isNarrow }"
          :checkout="checkoutUiReady ? activeCheckout : null"
          :repo="checkoutUiReady ? activeRepo : null"
          :git-snapshot="gitSnapshot"
          :review="review"
          :agent-sessions="agent.sessions"
          :agent-target-id="agent.targetId"
          :command-request="inspectorCommand"
          :saved-state="activeCheckout ? checkoutUiStates[activeCheckout.id] : null"
          @open-file="openFileDocument"
          @open-change="openChangedDocument"
          @select-agent-target="agent.selectTarget"
          @send-review="sendReviewToAgent"
          @update-ui-state="activeCheckout && updateInspectorUiState(activeCheckout.id, $event)"
        />
      </SplitterPanel>
    </SplitterGroup>
    <WorktreeDialog
      :open="!!lifecycle"
      :mode="lifecycle?.mode ?? 'create'"
      :repo="lifecycleRepo"
      :checkout="lifecycleCheckout"
      @close="lifecycle = null"
      @workspace-updated="applyWorkspace"
      @request-shell="requestShell"
      @warning="reportWarning"
    />
    <CommandPalette :commands="paletteCommands" :open-request-token="paletteRequestToken" @select="runPaletteCommand" />
    <div
      v-if="error"
      role="alert"
      class="absolute bottom-12 left-1/2 z-50 max-w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 rounded-lg border border-red-400/20 bg-[#242126] px-4 py-3 text-sm text-red-200 shadow-xl"
    >
      {{ error }}
    </div>
  </div>
  <div v-else class="h-full bg-transparent p-5 text-sm text-zinc-400" role="status">Restoring workspace layout…</div>
</template>
