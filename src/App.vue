<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, nextTick, onMounted, onUnmounted, provide, ref, watch } from "vue";
import {
  PopoverContent,
  PopoverRoot,
  PopoverTrigger,
  SplitterGroup,
  SplitterPanel,
  SplitterResizeHandle,
} from "reka-ui";
import {
  ChevronDown as ChevronDownIcon,
  GitFork as GitForkIcon,
  Search as SearchIcon,
  Settings as SettingsIcon,
} from "@lucide/vue";
import type { Checkout } from "./domain/workspace";
import { parseEditorPosition } from "./domain/editor";
import type { EditorPosition } from "./domain/editor";
import { mainViewFromState, mainViewLabel, mainViewToState, resolveMainView } from "./domain/main-document";
import type { DocumentMode, MainView } from "./domain/main-document";
import InspectorPane from "./components/InspectorPane.vue";
import MainPane from "./components/MainPane.vue";
import Sidebar from "./components/Sidebar.vue";
import ToastStack from "./components/ToastStack.vue";
import WorktreeDialog from "./components/WorktreeDialog.vue";
import type { TerminalSessionStatus, WorkspaceState } from "./domain/workspace";
import {
  getEditorAvailability,
  openInZed,
  closeMissingCheckout as persistMissingCheckoutClose,
  selectCheckout as persistCheckoutSelection,
} from "./lib/ipc";

import type { EditorAvailability } from "./lib/ipc";
import { useWorkspaceState } from "./presentation/workspace";
import { useActiveGitSnapshot } from "./presentation/active-git-snapshot";
import { REVIEW_SENDER, useReviewNotes } from "./presentation/review-notes";
import type { ReviewSender } from "./presentation/review-notes";
import { useAgentSessions } from "./presentation/agent-sessions";
import { useToasts } from "./presentation/toasts";
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
  chooseFolder,
  selectCheckout,
  selectSession: selectWorkspaceSession,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const { push: pushToast, pushCause: reportCause } = useToasts();
const appLayout = ref<AppLayoutState>({ ...DEFAULT_APP_LAYOUT });
const appLayoutReady = ref(false);
const checkoutUiStates = ref<Record<string, CheckoutUiState>>({});
const checkoutUiReady = ref(false);
const mainPane = ref<InstanceType<typeof MainPane> | null>(null);
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
const sendingReview = ref(false);
const mainViews = ref<Record<string, MainView>>({});
const editorAvailability = ref<EditorAvailability>({ zed: false, neovim: false });
const sessionRuntimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const documentRefreshRevisions = ref<Record<string, number>>({});
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
const activeMainView = computed(() => resolveMainView(mainViews.value, activeCheckout.value?.id ?? null));
/** The active session of the active checkout, or its last one. */
const activeSession = computed(() => {
  const checkout = activeCheckout.value;
  if (!checkout) return null;
  return (
    checkout.sessions.find((session) => session.id === workspace.value.activeSessionId) ??
    checkout.sessions.at(-1) ??
    null
  );
});
/** What the last crumb names. A file or a change set is not a session, so it is named as itself. */
const activeViewLabel = computed(() => mainViewLabel(activeMainView.value, activeSession.value?.name ?? null));
/** The file an external editor would open, or null when the view is the whole change set. */
const activeViewFile = computed(() => {
  const view = activeMainView.value;
  return view.kind === "terminal" ? null : view.path;
});
const activeCheckoutUiState = computed(() => {
  const checkoutId = activeCheckout.value?.id;
  return checkoutId ? checkoutUiStates.value[checkoutId] : undefined;
});

/** Shows one view in the main panel and saves it as this checkout's restored view. */
function showView(checkoutId: string, view: MainView) {
  const previous = mainViews.value[checkoutId];
  mainViews.value = { ...mainViews.value, [checkoutId]: view };
  updateCheckoutUiState(checkoutId, {
    ...mainViewToState(view, checkoutId),
    // A different file starts at the top: the offsets belong to what was read.
    ...(viewPath(previous) !== viewPath(view) && { documentScrollTop: 0, documentScrollLeft: 0, diffScrollTop: 0 }),
  });
}

function viewPath(view: MainView | undefined): string | null {
  if (!view || view.kind === "terminal") return null;
  return view.path;
}

function openFileDocument(selection: { checkoutId: string; path: string }) {
  showView(selection.checkoutId, {
    kind: "document",
    path: selection.path,
    mode: isMarkdownPath(selection.path) ? "view" : "code",
  });
}

function openChangedDocument(selection: { checkoutId: string; path: string }) {
  showView(selection.checkoutId, { kind: "diff", path: selection.path });
}

function openAllChanges(selection: { checkoutId: string }) {
  showView(selection.checkoutId, { kind: "diff", path: null });
}

function setDocumentMode(mode: DocumentMode) {
  const checkoutId = activeCheckout.value?.id;
  const view = activeMainView.value;
  if (!checkoutId || view.kind !== "document") return;
  showView(checkoutId, { ...view, mode });
}

function activateCheckoutTerminal(checkoutId: string) {
  // Selecting a checkout restores its saved main view. Only explicit terminal actions switch views.
  void selectCheckout(checkoutId);
}

async function activateTerminalSession(sessionId: string) {
  const checkout = allCheckouts.value.find((item) => item.sessions.some((session) => session.id === sessionId));
  if (!checkout) return;
  await selectWorkspaceSession(sessionId);
  showView(checkout.id, { kind: "terminal", sessionId });
  await nextTick();
  mainPane.value?.focusActiveTerminal();
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
      reportCause(cause);
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
        mainViews.value = { ...mainViews.value, [checkoutId]: mainViewFromState(state) };
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
    reportCause(cause);
    appLayout.value = { ...DEFAULT_APP_LAYOUT };
  } finally {
    appLayoutReady.value = true;
  }
  try {
    const dispose = await listen<string>("checkout-file-activity", (event) => {
      documentRefreshRevisions.value = {
        ...documentRefreshRevisions.value,
        [event.payload]: (documentRefreshRevisions.value[event.payload] ?? 0) + 1,
      };
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
  if (uiLayoutSaveTimer !== undefined) window.clearTimeout(uiLayoutSaveTimer);
  for (const timer of checkoutUiSaveTimers.values()) window.clearTimeout(timer);
  checkoutUiSaveTimers.clear();
});

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
}

function showWindowError(cause: unknown) {
  reportCause(cause);
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

/**
 * Takes a checkout whose directory is gone off the list, and nothing else.
 *
 * A repo root — the primary checkout of a Git repository, and the only checkout of a plain
 * folder — is registered as the head of a list, so closing one takes that list with it; a
 * worktree is an entry of its own. Marvis deletes no files either way, so the confirmation
 * only has to name the scope.
 */
async function closeMissingCheckout(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  if (!checkout) return;
  const scope = checkout.isPrimary ? " and its checkout list" : "";
  if (!window.confirm(`Close “${checkout.path}”${scope} in Marvis? No files will be deleted.`)) return;
  try {
    applyWorkspace(await persistMissingCheckoutClose(checkoutId));
  } catch (cause) {
    reportCause(cause);
  }
}

async function requestShell(checkoutId: string) {
  showView(checkoutId, { kind: "terminal", sessionId: null });
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
    shellRequest.value = { checkoutId, token: ++shellRequestToken };
  } catch (cause) {
    reportCause(cause);
  }
}

async function requestNvim(checkoutId: string, filePath?: string, position?: EditorPosition) {
  showView(checkoutId, { kind: "terminal", sessionId: null });
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
    reportCause(cause);
  }
}

/**
 * Hands the file the main panel is showing to an external editor. A line is asked for only when
 * there is a file to place the cursor in, so opening an editor without one takes no input.
 */
async function requestEditor(editor: "zed" | "neovim") {
  const checkout = activeCheckout.value;
  if (!checkout || checkout.isMissing) return;
  const file = activeViewFile.value ?? undefined;
  const position = file ? promptEditorPosition() : undefined;
  if (file && !position) return;
  if (editor === "neovim") {
    await requestNvim(checkout.id, file, position ?? undefined);
    return;
  }
  try {
    await openInZed(checkout.id, file, position?.line, position?.column);
  } catch (cause) {
    reportCause(cause);
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
    reportCause(cause);
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

/**
 * The send, as the diff sees it.
 *
 * `acked` is the only round whose turn is known to be over, so anything else is what the
 * "not finished" count is counting. The target keeps following the newest live session until
 * the user picks one, which `selectTarget` does.
 */
const reviewSender: ReviewSender = {
  get sessions() {
    return agent.sessions;
  },
  get targetId() {
    return agent.targetId;
  },
  get unfinishedRounds() {
    return review.rounds.filter((round) => round.status !== "acked").length;
  },
  selectTarget: (sessionId) => agent.selectTarget(sessionId),
  send: sendReviewToAgent,
};
provide(REVIEW_SENDER, reviewSender);

function promptEditorPosition(): EditorPosition | null {
  const value = window.prompt("Open selected file at line[:column]:", "1");
  if (value === null) return null;
  const position = parseEditorPosition(value);
  if (!position) pushToast("Enter a positive line number with an optional column (for example, 42:7).");
  return position;
}

function updateSessionStatus(sessionId: string, status: TerminalSessionStatus | null) {
  if (status) sessionRuntimeStatuses.value[sessionId] = status;
  else delete sessionRuntimeStatuses.value[sessionId];
}

async function closeTerminalSession(sessionId: string) {
  await mainPane.value?.requestClose(sessionId);
}

function applyWorkspace(next: WorkspaceState) {
  updateWorkspace(next);
}

function reportWarning(message: string) {
  pushToast(message, "info");
}
</script>

<template>
  <div
    v-if="appLayoutReady"
    class="app-shell relative flex h-full min-w-[900px] flex-col"
    :style="{ '--inspector-width': `${appLayout.inspectorWidth}px` }"
  >
    <header class="window-header flex h-12 shrink-0 items-center gap-4 border-b pl-[78px] pr-4">
      <!-- The mockup's field is icon + placeholder on one flat surface, not a bordered box. -->
      <div class="flex w-[min(300px,34vw)] min-w-[220px] shrink-0 items-center gap-2 bg-(--marvis-bg-2) px-2.5 py-1.5">
        <SearchIcon class="icon-sm shrink-0 text-(--marvis-text-faint)" aria-hidden="true" />
        <input
          ref="searchField"
          type="search"
          data-testid="search-field"
          aria-label="Search files and commands"
          placeholder="Search..."
          class="window-search min-w-0 flex-1 appearance-none bg-transparent p-0 text-xs text-(--marvis-text-secondary) outline-none placeholder:text-(--marvis-text-faint)"
          @keydown.esc="blurSearchField"
        />
      </div>
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
          <!-- The last crumb names the view that is open. A file or a change set is picked from
               the details panel, so it is plain text there: a session dropdown over a diff
               would name something that is not on screen. -->
          <PopoverRoot v-if="activeMainView.kind === 'terminal'">
            <PopoverTrigger
              data-testid="item-crumb"
              class="marvis-control min-w-0 text-xs text-(--marvis-text) hover:text-(--marvis-text)"
              :title="activeViewLabel"
            >
              <span class="truncate">{{ activeViewLabel }}</span>
              <ChevronDownIcon class="icon-xs shrink-0 text-(--marvis-text-faint)" aria-hidden="true" />
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
          <span v-else data-testid="item-crumb" class="min-w-0 truncate text-(--marvis-text)" :title="activeViewLabel">
            {{ activeViewLabel }}
          </span>
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
          :session-runtime-statuses="sessionRuntimeStatuses"
          :is-opening="isOpening"
          @open-folder="chooseFolder"
          @select-checkout="activateCheckoutTerminal"
          @select-session="activateTerminalSession"
          @create-worktree="openWorktreeDialog('create', $event)"
          @new-terminal="requestShell"
          @remove-worktree="openWorktreeDialog('remove', $event)"
          @close-missing="closeMissingCheckout"
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
        <MainPane
          ref="mainPane"
          :checkout="activeCheckout"
          :view="activeMainView"
          :ready="checkoutUiReady"
          :git-snapshot="gitSnapshot"
          :review="review"
          :active-session-id="workspace.activeSessionId"
          :is-opening="isOpening"
          :shell-request="shellRequest"
          :nvim-request="nvimRequest"
          :registered-session-ids="registeredSessionIds"
          :zed-available="editorAvailability.zed"
          :neovim-available="editorAvailability.neovim"
          :refresh-revision="documentRefreshRevisions[activeCheckout?.id ?? ''] ?? 0"
          :reading-position="{
            top: activeCheckoutUiState?.documentScrollTop ?? 0,
            left: activeCheckoutUiState?.documentScrollLeft ?? 0,
          }"
          :diff-scroll-top="activeCheckoutUiState?.diffScrollTop ?? 0"
          @open-folder="chooseFolder"
          @workspace-updated="updateWorkspace"
          @session-status-changed="updateSessionStatus"
          @update-document-mode="setDocumentMode"
          @reading-position-changed="activeCheckout && updateDocumentReadingPosition(activeCheckout.id, $event)"
          @diff-position-changed="activeCheckout && updateDiffReadingPosition(activeCheckout.id, $event)"
          @open-markdown-link="activeCheckout && openFileDocument({ checkoutId: activeCheckout.id, path: $event })"
          @open-in-zed="requestEditor('zed')"
          @open-in-neovim="requestEditor('neovim')"
        />
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
        <!-- The review, the agent list and the send itself belong to the diff (F.5): this panel
             is the file tree and the change set, and nothing more. -->
        <InspectorPane
          :class="{ 'right-inspector-drawer': isNarrow }"
          :checkout="checkoutUiReady ? activeCheckout : null"
          :repo="checkoutUiReady ? activeRepo : null"
          :git-snapshot="gitSnapshot"
          :saved-state="activeCheckout ? checkoutUiStates[activeCheckout.id] : null"
          @open-file="openFileDocument"
          @open-change="openChangedDocument"
          @open-all-changes="openAllChanges"
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
    <ToastStack />
  </div>
  <div v-else class="h-full bg-transparent p-5 text-sm text-zinc-400" role="status">Restoring workspace layout…</div>
</template>
