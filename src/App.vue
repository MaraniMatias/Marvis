<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import {
  PopoverContent,
  PopoverRoot,
  PopoverTrigger,
  SplitterGroup,
  SplitterPanel,
  SplitterResizeHandle,
} from "reka-ui";
import { ChevronDown as ChevronDownIcon, GitFork as GitForkIcon, Settings as SettingsIcon } from "@lucide/vue";
import type { Checkout } from "./domain/workspace";
import { parseEditorPosition } from "./domain/editor";
import type { EditorPosition } from "./domain/editor";
import { resolveMainView } from "./domain/main-document";
import type { MainDocument, MainDocumentMode } from "./domain/main-document";
import InspectorPane from "./components/InspectorPane.vue";
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
  INSPECTOR_WIDTH_LIMITS,
  SIDEBAR_WIDTH_LIMITS,
  needsInspectorDrawer,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  resizeLayoutPanel,
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
const sidebarPanel = ref<{ resize(size: number): void } | null>(null);
const inspectorPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const searchField = ref<HTMLInputElement | null>(null);
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
/** What the titlebar names: the active session of the active checkout, or its last one. */
const activeItem = computed(() => {
  const checkout = activeCheckout.value;
  if (!checkout) return null;
  return (
    checkout.sessions.find((session) => session.id === workspace.value.activeSessionId) ??
    checkout.sessions.at(-1) ??
    null
  );
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

function onSplitterLayout(sizes: number[]) {
  if (!appLayoutReady.value || sizes.length < 3) return;
  let next = appLayout.value;
  if (sizes[0] > 0) next = resizeLayoutPanel(next, "sidebar", sizes[0]);
  if (!isNarrow.value && sizes[2] > 0) next = resizeLayoutPanel(next, "inspector", sizes[2]);
  if (next.sidebarWidth !== appLayout.value.sidebarWidth || next.inspectorWidth !== appLayout.value.inspectorWidth) {
    appLayout.value = next;
  }
}

function resetPanelWidth(panel: "sidebar" | "inspector") {
  const width = panel === "sidebar" ? DEFAULT_APP_LAYOUT.sidebarWidth : DEFAULT_APP_LAYOUT.inspectorWidth;
  (panel === "sidebar" ? sidebarPanel.value : inspectorPanel.value)?.resize(width);
  appLayout.value = resizeLayoutPanel(appLayout.value, panel, width);
}

watch(appLayout, () => scheduleAppLayoutSave(), { deep: true });

// A narrow window cannot hold the main panel and the inspector side by side, so the inspector
// floats over it as a drawer. Its width is left alone, to be restored when space returns.
watch(
  [isNarrow, appLayoutReady],
  async () => {
    if (!appLayoutReady.value) return;
    await nextTick();
    if (isNarrow.value) inspectorPanel.value?.collapse();
    else {
      inspectorPanel.value?.expand();
      inspectorPanel.value?.resize(appLayout.value.inspectorWidth);
    }
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

/** Double-clicking the empty part of the title bar zooms the window, as the platform does. */
function zoomFromTitlebar() {
  void getCurrentWindow().toggleMaximize().catch(showWindowError);
}

function blurSearchField() {
  searchField.value?.blur();
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
 * Hands the active document to an external editor. A line is asked for only when there is a
 * file to place the cursor in, so opening an editor without a document takes no input.
 */
async function requestEditor(editor: "zed" | "neovim") {
  const checkout = activeCheckout.value;
  if (!checkout || checkout.isMissing) return;
  const file = activeDocument.value?.checkoutId === checkout.id ? activeDocument.value.path : undefined;
  const position = file ? promptEditorPosition() : undefined;
  if (file && !position) return;
  if (editor === "neovim") {
    await requestNvim(checkout.id, file, position ?? undefined);
    return;
  }
  try {
    await openInZed(checkout.id, file, position?.line, position?.column);
  } catch (cause) {
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

function updateSessionStatus(sessionId: string, status: TerminalSessionStatus | null) {
  if (status) sessionRuntimeStatuses.value[sessionId] = status;
  else delete sessionRuntimeStatuses.value[sessionId];
}

async function closeTerminalSession(sessionId: string) {
  await sessionPane.value?.requestClose(sessionId);
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
    class="app-shell relative flex h-full min-w-[900px] flex-col"
    :style="{ '--inspector-width': `${appLayout.inspectorWidth}px` }"
  >
    <header class="window-header flex h-12 shrink-0 items-center gap-4 border-b pl-[78px] pr-4">
      <!-- The native traffic lights own the first 78px; the inset leaves them room to breathe. -->
      <input
        ref="searchField"
        type="search"
        data-testid="search-field"
        aria-label="Search files and commands"
        placeholder="Search..."
        class="window-search h-7 w-[min(300px,34vw)] shrink-0 appearance-none rounded border border-transparent px-2.5 text-xs placeholder:text-(--marvis-text-faint)"
        @keydown.esc="blurSearchField"
      />
      <!-- Keep native dragging and double-click zoom on this empty spacer, clear of controls. -->
      <div data-tauri-drag-region aria-hidden="true" class="h-full min-w-4 flex-1" @dblclick="zoomFromTitlebar" />
      <nav
        aria-label="Repository location"
        class="window-breadcrumb flex h-full min-w-0 shrink items-center gap-1.5 text-xs"
      >
        <template v-if="activeCheckout">
          <span data-testid="repo-crumb" class="max-w-40 truncate text-(--marvis-text)">
            {{ activeRepo?.name ?? activeCheckout.path.split(/[\\/]/).at(-1) }}
          </span>
          <template v-if="activeRepo?.kind === 'git'">
            <span aria-hidden="true" class="text-(--marvis-text-faint)">/</span>
            <GitForkIcon class="icon-xs shrink-0" aria-hidden="true" />
            <span class="max-w-32 truncate text-(--marvis-text-secondary)">
              {{ activeCheckout.branch || "Detached" }}
            </span>
          </template>
          <span aria-hidden="true" class="text-(--marvis-text-faint)">/</span>
          <PopoverRoot>
            <PopoverTrigger
              data-testid="item-crumb"
              class="flex min-w-0 items-center gap-1 text-(--marvis-text)"
              :title="activeItem?.name"
            >
              <span class="truncate">{{ activeItem?.name ?? "Terminal" }}</span>
              <ChevronDownIcon class="icon-xs shrink-0" aria-hidden="true" />
            </PopoverTrigger>
            <PopoverContent
              side="bottom"
              align="end"
              :side-offset="4"
              class="surface-popover flex min-w-40 flex-col rounded p-1 text-xs text-(--marvis-text)"
            >
              <button
                v-for="session in activeCheckout.sessions"
                :key="session.id"
                type="button"
                class="rounded px-2 py-1 text-left hover:bg-(--marvis-bg-2)"
                @click="activateTerminalSession(session.id)"
              >
                {{ session.name }}
              </button>
              <span v-if="!activeCheckout.sessions.length" class="px-2 py-1 text-(--marvis-text-faint)">
                No open terminals
              </span>
            </PopoverContent>
          </PopoverRoot>
        </template>
      </nav>
      <button
        type="button"
        aria-label="Settings"
        data-testid="settings-button"
        class="shrink-0 text-(--marvis-text-secondary) hover:text-(--marvis-text)"
      >
        <SettingsIcon class="icon-xs" aria-hidden="true" />
      </button>
    </header>
    <SplitterGroup direction="horizontal" class="app-splitter flex min-h-0 flex-1" @layout="onSplitterLayout">
      <SplitterPanel
        id="navigation-panel"
        ref="sidebarPanel"
        :default-size="appLayout.sidebarWidth"
        :min-size="SIDEBAR_WIDTH_LIMITS.min"
        :max-size="SIDEBAR_WIDTH_LIMITS.max"
        size-unit="px"
        class="min-h-0 shrink-0"
      >
        <Sidebar
          :repos="workspace.repos"
          :active-checkout-id="workspace.activeCheckoutId"
          :active-session-id="workspace.activeSessionId"
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
        />
      </SplitterPanel>
      <SplitterResizeHandle
        id="navigation-resize-handle"
        aria-label="Resize navigation sidebar"
        class="splitter-handle"
        @dblclick.stop="resetPanelWidth('sidebar')"
      >
        <div
          aria-hidden="true"
          class="absolute left-1/2 top-1/2 h-6 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--marvis-text-faint)"
        />
      </SplitterResizeHandle>
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
              @open-in-zed="requestEditor('zed')"
            />
          </section>
        </div>
      </SplitterPanel>
      <SplitterResizeHandle
        id="inspector-resize-handle"
        aria-label="Resize files and changes inspector"
        class="splitter-handle"
        @dblclick.stop="resetPanelWidth('inspector')"
      >
        <div
          aria-hidden="true"
          class="absolute left-1/2 top-1/2 h-6 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--marvis-text-faint)"
        />
      </SplitterResizeHandle>
      <SplitterPanel
        id="inspector-panel"
        ref="inspectorPanel"
        :default-size="isNarrow ? 0 : appLayout.inspectorWidth"
        :min-size="INSPECTOR_WIDTH_LIMITS.min"
        :max-size="INSPECTOR_WIDTH_LIMITS.max"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="inspector-splitter-panel relative min-h-0 shrink-0 overflow-visible"
      >
        <InspectorPane
          :class="{ 'right-inspector-drawer': isNarrow }"
          :checkout="checkoutUiReady ? activeCheckout : null"
          :repo="checkoutUiReady ? activeRepo : null"
          :git-snapshot="gitSnapshot"
          :review="review"
          :agent-sessions="agent.sessions"
          :agent-target-id="agent.targetId"
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
  </div>
  <div v-else class="h-full bg-transparent p-5 text-sm text-zinc-400" role="status">Restoring workspace layout…</div>
</template>
