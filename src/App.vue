<script setup lang="ts">
/* eslint-disable vue/html-closing-bracket-newline, vue/html-indent */
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, nextTick, onMounted, onUnmounted, provide, ref, watch } from "vue";
import { SplitterGroup, SplitterPanel, SplitterResizeHandle } from "reka-ui";
import {
  Columns2 as Columns2Icon,
  Columns3 as Columns3Icon,
  GitFork as GitForkIcon,
  Settings as SettingsIcon,
} from "@lucide/vue";
import type { Checkout } from "./domain/workspace";
import { mainViewFromState, mainViewLabel, mainViewToState, resolveMainView } from "./domain/main-document";
import type { DocumentMode, MainView } from "./domain/main-document";
import type { TitlebarMenuItem, TitlebarMenuSection } from "./domain/titlebar-menu";
import InspectorPane from "./components/InspectorPane.vue";
import MainPane from "./components/MainPane.vue";
import Sidebar from "./components/Sidebar.vue";
import TitlebarMenu from "./components/TitlebarMenu.vue";
import ToastStack from "./components/ToastStack.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import WorktreeDialog from "./components/WorktreeDialog.vue";
import type { TerminalSessionStatus, WorkspaceState } from "./domain/workspace";
import { workdirTitle } from "./domain/workspace";
import {
  archiveCheckout as persistCheckoutArchive,
  closeCheckout as persistCheckoutClose,
  closeMissingCheckout as persistMissingCheckoutClose,
  exportReviewMarkdown,
  loadReviewTarget,
  renameTerminal,
  restoreArchivedWorktrees as persistArchivedRestore,
  selectCheckout as persistCheckoutSelection,
  saveReviewTarget,
} from "./lib/ipc";

import { useWorkspaceState } from "./presentation/workspace";
import { useRecentPaths } from "./presentation/recent-paths";
import { useActiveGitSnapshot } from "./presentation/active-git-snapshot";
import { useGitWatchers } from "./presentation/git-watchers";
import { useWorktreeSync } from "./presentation/worktree-sync";
import { REVIEW_SENDER, useReviewNotes } from "./presentation/review-notes";
import type { ReviewSender, ReviewTarget } from "./presentation/review-notes";
import { useAgentSessions } from "./presentation/agent-sessions";
import { useToasts } from "./presentation/toasts";
import { defaultAgentSession } from "./domain/agent";
import { buildReviewMarkdown, localReviewTimestamp } from "./domain/review";
import { isMarkdownPath } from "./presentation/markdown-preview";
import {
  DEFAULT_APP_LAYOUT,
  DEFAULT_CHECKOUT_UI_STATE,
  INSPECTOR_WIDTH_LIMITS,
  SIDEBAR_WIDTH_LIMITS,
  TERMINAL_SCROLLBAR_MODES,
  needsInspectorDrawer,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  resizeLayoutPanel,
} from "./domain/ui-state";
import type { AppLayoutState, CheckoutUiState, TerminalScrollbarMode } from "./domain/ui-state";
import { loadAppLayout, loadCheckoutUiState, saveAppLayout, saveCheckoutUiState } from "./lib/ipc";

const {
  workspace,
  activeCheckout,
  isOpening,
  chooseFolder,
  openPath,
  selectCheckout,
  selectSession: selectWorkspaceSession,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const { recentPaths, refresh: refreshRecentPaths } = useRecentPaths();
/** The one crumb menu that is open: opening any of them closes the other two. */
const openCrumb = ref<string | null>(null);
const { push: pushToast, pushCause: reportCause } = useToasts();
const appLayout = ref<AppLayoutState>({ ...DEFAULT_APP_LAYOUT });
const appLayoutReady = ref(false);
const appShell = ref<HTMLElement | null>(null);
const checkoutUiStates = ref<Record<string, CheckoutUiState>>({});
const checkoutUiReady = ref(false);
const mainPane = ref<InstanceType<typeof MainPane> | null>(null);
const sidebarPanel = ref<{ resize(size: number): void } | null>(null);
const inspectorPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const viewportWidth = ref(window.innerWidth);
const isNarrow = computed(
  () => appLayout.value.mode === "focus" && needsInspectorDrawer(appLayout.value, viewportWidth.value),
);
const isSplitLayout = computed(() => appLayout.value.mode === "split");
const inspectorInDrawer = computed(() => isNarrow.value || isSplitLayout.value);
const splitInspectorOpen = ref(false);
const pointerOverSplitStrip = ref(false);
const pointerOverSplitDrawer = ref(false);
const activeRepo = computed(
  () =>
    workspace.value.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === activeCheckout.value?.id)) ??
    null,
);
// The sidebar names every checkout at once, so every one of them needs a change signal and not
// only the one on screen.
const unwatchedRepos = useGitWatchers(() => workspace.value.repos);
const gitSnapshot = useActiveGitSnapshot(
  activeCheckout,
  activeRepo,
  () => void promptForDefaultBranchIfNeeded(true),
  unwatchedRepos,
);
const review = useReviewNotes(activeCheckout, activeRepo);
const agent = useAgentSessions(activeCheckout, activeRepo);
const reviewTarget = ref<ReviewTarget>("markdown");
let reviewTargetLoadGeneration = 0;
watch(
  () => activeCheckout.value?.id ?? null,
  async (checkoutId) => {
    const request = ++reviewTargetLoadGeneration;
    reviewTarget.value = "markdown";
    if (!checkoutId) return;
    try {
      const target = await loadReviewTarget(checkoutId);
      if (request === reviewTargetLoadGeneration) reviewTarget.value = target === "opencode" ? target : "markdown";
    } catch (cause) {
      if (request === reviewTargetLoadGeneration) reportCause(cause);
    }
  },
  { immediate: true },
);
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

// The recents are read when the workdir menu is about to be read, so a folder opened a moment
// ago is already in the list. A failure is a toast, and the menu opens without the group.
watch(openCrumb, (name) => {
  if (name === "workdir") void refreshRecentPaths();
});
const sendingReview = ref(false);
const mainViews = ref<Record<string, MainView>>({});
const sessionRuntimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const documentRefreshRevisions = ref<Record<string, number>>({});
let shellRequestToken = 0;
let unlistenFileActivity: (() => void) | undefined;
let activityListenerDisposed = false;
let unlistenCloseRequested: (() => void) | undefined;
let allowWindowClose = false;
let windowClosePromise: Promise<void> | null = null;
let uiLayoutSaveTimer: number | undefined;
let inspectorCloseTimer: number | undefined;
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
/**
 * Whether there is a last crumb at all.
 *
 * It names what the panel is showing, so a workdir with no terminal has nothing to name there:
 * the crumb would read "Terminal" over a panel that says it has none. The file and change set
 * views always have a name, so only the empty terminal leaves the line without its last step.
 */
const hasItemCrumb = computed(() => activeMainView.value.kind !== "terminal" || activeSession.value !== null);
/**
 * The last crumb of a file, in the steps it is made of.
 *
 * A path is already a line of crumbs, so it is drawn as one: the directories and the file name
 * with the same separator the rest of the line uses, instead of one run of text with slashes
 * buried in it. A label that is not a path — a session, the whole change set — is one step.
 */
const pathSteps = computed(() => activeViewLabel.value.split("/").filter(Boolean));

const lineEl = ref<HTMLElement | null>(null);
const pathCrumbEl = ref<HTMLElement | null>(null);
const pathProbeEl = ref<HTMLElement | null>(null);
const pathElided = ref(false);
/**
 * The path as the crumb draws it: every step, or the two ends with a rule where the rest was.
 *
 * One list either way, so the separators, their colour and their space are the same code for
 * the whole shape — the elided crumb is not a different crumb, it is the same crumb with less
 * in it, and the two ends are what the reader came for.
 */
const drawnSteps = computed(() => {
  if (!pathElided.value) return pathSteps.value;
  const first = pathSteps.value[0]!;
  const last = pathSteps.value.at(-1)!;
  return first === last ? [first] : [first, "…", last];
});

/**
 * Whether the whole path fits in the room the line still has for it.
 *
 * The line is bounded by a share of the window, so the room is that share minus everything the
 * path does not get to keep: the workdir, the branch, the fork and the separators. The probe is
 * the path at its natural width and it is always mounted, so this weighs two numbers that do not
 * move when the shape on screen does — otherwise an elided path would measure itself as the one
 * that fitted and the line could never come back.
 *
 * Only a path with something in the middle can lose it, and the two ends are what stay: the
 * first directory says where you are, the file name is what you came to see.
 */
function measurePath() {
  const line = lineEl.value;
  const crumb = pathCrumbEl.value;
  const probe = pathProbeEl.value;
  if (!line || !crumb || !probe) return;
  const style = window.getComputedStyle(line);
  const header = line.parentElement?.clientWidth ?? 0;
  const share = style.maxWidth.endsWith("%")
    ? (parseFloat(style.maxWidth) / 100) * header
    : parseFloat(style.maxWidth) || header;
  const others = [...line.children].reduce(
    (width, child) => (child === crumb || child === probe ? width : width + child.getBoundingClientRect().width),
    0,
  );
  const gaps = (line.childElementCount - 1) * (parseFloat(style.columnGap) || 0);
  pathElided.value = pathSteps.value.length > 2 && probe.scrollWidth > share - others - gaps;
}

// The path is weighed after the DOM settles, and again whenever the window changes, because the
// room it has is what the window and the rest of the line leave it.
watch(
  () => [activeViewLabel.value, activeCheckout.value?.id],
  () => void nextTick(measurePath),
);

/**
 * The three crumb menus: the workdir line of the titlebar is the sidebar read sideways.
 *
 * Each crumb names one level of it — the workdir, its branch or worktree, and the item open
 * inside it — and each opens the list of the level it names. What is not in this window is a
 * matter of record, not of guessing, so a workdir whose directory is gone is listed dimmed
 * and cannot be picked.
 */
const workdirMenu = computed<TitlebarMenuSection[]>(() => {
  const open = allCheckouts.value;
  const openPaths = new Set(open.map((checkout) => checkout.canonicalPath));
  // A folder already open in this window belongs to This Window, not to the recents that
  // remember it: the same workdir listed twice would be two ways into one place.
  const recents: TitlebarMenuItem[] = recentPaths.value
    .filter((recent) => !openPaths.has(recent.canonicalPath))
    .map((recent) => ({
      id: `recent:${recent.canonicalPath}`,
      label: recent.canonicalPath.split(/[\\/]/).at(-1) || recent.canonicalPath,
      hint: recent.canonicalPath,
      title: recent.canonicalPath,
      run: () => void openPath(recent.canonicalPath),
    }));
  return [
    {
      kind: "group",
      label: "This Window",
      items: open.map((checkout) => workdirItem(checkout, `workdir:${checkout.id}`, repoName(checkout))),
    },
    ...(recents.length ? [{ kind: "group" as const, label: "Recent Projects", items: recents }] : []),
    { kind: "separator" },
    { kind: "list", items: [{ id: "open-directory", label: "Open directory", pinned: true, run: chooseFolder }] },
  ];
});

/** The worktrees of the repo the active workdir belongs to, and the one that makes another. */
const worktreeMenu = computed<TitlebarMenuSection[]>(() => {
  const repo = activeRepo.value;
  if (repo?.kind !== "git") return [];
  // The worktree is added to the repo's root, wherever the active workdir is, so the action
  // names the root and the dialog resolves its defaults from it.
  const root = repo.checkouts.find((checkout) => checkout.isPrimary);
  return [
    {
      kind: "group",
      label: "Worktrees",
      // A branch names one worktree, so nothing has to tell two rows apart here: the path is
      // in the row's tooltip, where it does not cost the name its width.
      items: repo.checkouts.map((checkout) => workdirItem(checkout, `worktree:${checkout.id}`)),
    },
    { kind: "separator" },
    {
      kind: "list",
      items: root
        ? [
            {
              id: "new-worktree",
              label: "New worktree",
              pinned: true,
              run: () => openWorktreeDialog("create", root.id),
            },
          ]
        : [],
    },
  ];
});

/**
 * What the three scrollbar modes are called, in the order they are offered. The names are the
 * behaviour rather than a description of it, because the behaviour is the thing being chosen and
 * "Always" next to "Auto" is read correctly by somebody who has never seen a settings pane about
 * scrollbars before.
 */
const TERMINAL_SCROLLBAR_LABELS: Record<TerminalScrollbarMode, string> = {
  hidden: "Hidden",
  auto: "Auto",
  always: "Always",
};

function setTerminalScrollbar(mode: TerminalScrollbarMode) {
  if (appLayout.value.terminalScrollbar === mode) return;
  appLayout.value = { ...appLayout.value, terminalScrollbar: mode };
}

/** The subitems of the active workdir: the terminals it has open, and how to start another. */
const terminalMenu = computed<TitlebarMenuSection[]>(() => {
  const checkout = activeCheckout.value;
  if (!checkout) return [];
  const sessions: TitlebarMenuItem[] = checkout.sessions.map((session) => ({
    id: session.id,
    label: session.name,
    title: session.name,
    checked: session.id === workspace.value.activeSessionId,
    run: () => void activateTerminalSession(session.id),
  }));
  return [
    { kind: "group", label: "Terminals", items: sessions },
    { kind: "separator" },
    {
      kind: "group",
      label: "Scrollbar",
      // The word is on every row rather than only on the group header because the header is not
      // searched: typing "scrollbar" into the menu has to leave these three standing, and a
      // header alone would filter the group away and answer the question with an empty menu.
      items: TERMINAL_SCROLLBAR_MODES.map((mode) => ({
        id: `terminal-scrollbar-${mode}`,
        label: TERMINAL_SCROLLBAR_LABELS[mode],
        hint: "Scrollbar",
        checked: appLayout.value.terminalScrollbar === mode,
        choice: true,
        run: () => setTerminalScrollbar(mode),
      })),
    },
    { kind: "separator" },
    {
      kind: "list",
      items: [{ id: "new-terminal", label: "New terminal", pinned: true, run: () => requestShell(checkout.id) }],
    },
  ];
});

/** What the workdir crumb says: the repo, or the folder when there is no repo to name. */
const workdirLabel = computed(() => activeRepo.value?.name ?? activeCheckout.value?.path.split(/[\\/]/).at(-1) ?? "");

function repoName(checkout: Checkout): string {
  return workspace.value.repos.find((repo) => repo.id === checkout.repoId)?.name ?? checkout.path;
}

/** One crumb menu at a time: the name it reports becomes the one that is open. */
function setCrumbOpen(name: string, open: boolean) {
  openCrumb.value = open ? name : null;
}

function workdirItem(checkout: Checkout, id: string, hint?: string): TitlebarMenuItem {
  return {
    id,
    label: workdirTitle(checkout),
    ...(hint !== undefined && { hint }),
    title: checkout.path,
    checked: checkout.id === workspace.value.activeCheckoutId,
    disabled: checkout.isMissing,
    run: () => void activateCheckoutTerminal(checkout.id),
  };
}
const activeCheckoutUiState = computed(() => {
  const checkoutId = activeCheckout.value?.id;
  return checkoutId ? checkoutUiStates.value[checkoutId] : undefined;
});

/** Shows one view in the main panel and saves it as this checkout's restored view. */
function showView(checkoutId: string, view: MainView) {
  const previous = mainViews.value[checkoutId];
  if (isSplitLayout.value && view.kind === "terminal" && previous && previous.kind !== "terminal") return;
  mainViews.value = { ...mainViews.value, [checkoutId]: view };
  updateCheckoutUiState(checkoutId, {
    ...mainViewToState(view, checkoutId),
    // A different file starts at the top: the offsets belong to what was read.
    ...(viewPath(previous) !== viewPath(view) && { documentScrollTop: 0, documentScrollLeft: 0, diffScrollTop: 0 }),
  });
}

function viewPath(view: MainView | undefined): string | null {
  if (!view || view.kind === "terminal") return null;
  return view.kind === "document" ? `${view.origin}\0${view.path}` : view.path;
}

function openFileDocument(selection: { checkoutId: string; path: string }) {
  showView(selection.checkoutId, {
    kind: "document",
    path: selection.path,
    mode: isMarkdownPath(selection.path) ? "view" : "code",
    origin: "checkout",
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

async function activateCheckoutTerminal(checkoutId: string, hasChanges = false) {
  // A row that shows `+N -N` is advertising a diff, so picking it opens the whole change set
  // for that workdir rather than whatever view it had saved. A row with nothing changed keeps
  // restoring its saved view, and no terminal is started either way.
  if (!hasChanges) {
    void selectCheckout(checkoutId);
    return;
  }
  await selectCheckout(checkoutId);
  showView(checkoutId, { kind: "diff", path: null });
}

async function activateTerminalSession(sessionId: string) {
  const checkout = allCheckouts.value.find((item) => item.sessions.some((session) => session.id === sessionId));
  if (!checkout) return;
  await selectWorkspaceSession(sessionId);
  if (!isSplitLayout.value) showView(checkout.id, { kind: "terminal", sessionId });
  else if (loadedCheckoutUiIds.has(checkout.id) && mainViews.value[checkout.id]?.kind === "terminal") {
    showView(checkout.id, { kind: "terminal", sessionId });
  }
  await nextTick();
  mainPane.value?.focusActiveTerminal();
}

function toggleLayoutMode() {
  appLayout.value = { ...appLayout.value, mode: appLayout.value.mode === "split" ? "focus" : "split" };
}

function cancelSplitInspectorClose() {
  if (inspectorCloseTimer !== undefined) window.clearTimeout(inspectorCloseTimer);
  inspectorCloseTimer = undefined;
}

function openSplitInspector() {
  if (!isSplitLayout.value) return;
  cancelSplitInspectorClose();
  splitInspectorOpen.value = true;
}

/**
 * The drawer closes only once the pointer is over neither the strip nor the drawer.
 *
 * The two live in different subtrees, so moving between them fires a leave on one and an
 * enter on the other. A timer alone would depend on the enter landing after the leave;
 * tracking where the pointer is makes the close a fact about the pointer instead.
 */
function scheduleSplitInspectorClose() {
  if (!isSplitLayout.value) return;
  if (pointerOverSplitStrip.value || pointerOverSplitDrawer.value) return;
  cancelSplitInspectorClose();
  inspectorCloseTimer = window.setTimeout(() => {
    inspectorCloseTimer = undefined;
    splitInspectorOpen.value = false;
  }, 300);
}

function enterSplitStrip() {
  pointerOverSplitStrip.value = true;
  openSplitInspector();
}

function leaveSplitStrip() {
  pointerOverSplitStrip.value = false;
  scheduleSplitInspectorClose();
}

function enterSplitDrawer() {
  pointerOverSplitDrawer.value = true;
  openSplitInspector();
}

function leaveSplitDrawer() {
  pointerOverSplitDrawer.value = false;
  scheduleSplitInspectorClose();
}

function closeSplitInspector() {
  cancelSplitInspectorClose();
  pointerOverSplitStrip.value = false;
  pointerOverSplitDrawer.value = false;
  splitInspectorOpen.value = false;
}

function onAppKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && splitInspectorOpen.value) closeSplitInspector();
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
  if (!inspectorInDrawer.value && sizes[2] > 0) next = resizeLayoutPanel(next, "inspector", sizes[2]);
  if (next.sidebarWidth !== appLayout.value.sidebarWidth || next.inspectorWidth !== appLayout.value.inspectorWidth) {
    appLayout.value = next;
  }
}

function resetPanelWidth(panel: "sidebar" | "inspector") {
  const width = panel === "sidebar" ? DEFAULT_APP_LAYOUT.sidebarWidth : DEFAULT_APP_LAYOUT.inspectorWidth;
  (panel === "sidebar" ? sidebarPanel.value : inspectorPanel.value)?.resize(width);
  appLayout.value = resizeLayoutPanel(appLayout.value, panel, width);
}

function resizeAppPreview(width: number) {
  appLayout.value = resizeLayoutPanel(appLayout.value, "preview", width);
}

watch(appLayout, () => scheduleAppLayoutSave(), { deep: true });

// A narrow window cannot hold the main panel and the inspector side by side, so the inspector
// floats over it as a drawer. Its width is left alone, to be restored when space returns.
watch(
  [inspectorInDrawer, appLayoutReady],
  async () => {
    if (!appLayoutReady.value) return;
    await nextTick();
    if (inspectorInDrawer.value) inspectorPanel.value?.collapse();
    else {
      inspectorPanel.value?.expand();
      inspectorPanel.value?.resize(appLayout.value.inspectorWidth);
    }
  },
  { immediate: true, flush: "post" },
);

watch(isSplitLayout, (split) => {
  if (!split) closeSplitInspector();
});

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
  window.addEventListener("keydown", onAppKeydown);
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
    const dispose = await listen<string[]>("checkout-file-activity", (event) => {
      // One watcher covers a whole repository, so a write in one worktree arrives naming the
      // worktree it landed in rather than the one that happened to be on screen.
      for (const checkoutId of event.payload) {
        documentRefreshRevisions.value = {
          ...documentRefreshRevisions.value,
          [checkoutId]: (documentRefreshRevisions.value[checkoutId] ?? 0) + 1,
        };
      }
    });
    if (activityListenerDisposed) dispose();
    else unlistenFileActivity = dispose;
  } catch {
    // File-write activity is optional; PTY foreground-process activity remains observable.
  }
});

onUnmounted(() => {
  activityListenerDisposed = true;
  unlistenCloseRequested?.();
  window.removeEventListener("resize", onViewportResize);
  window.removeEventListener("keydown", onAppKeydown);
  unlistenFileActivity?.();
  if (inspectorCloseTimer !== undefined) window.clearTimeout(inspectorCloseTimer);
  if (uiLayoutSaveTimer !== undefined) window.clearTimeout(uiLayoutSaveTimer);
  for (const timer of checkoutUiSaveTimers.values()) window.clearTimeout(timer);
  checkoutUiSaveTimers.clear();
});

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
  measurePath();
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

/**
 * Takes a workdir off the panel, and leaves the disk alone.
 *
 * This is the row's own action, so the confirmation has to say what the row cannot: nothing is
 * deleted, and the directory is still there to open. A repo root — the primary checkout of a
 * Git repository, and the only checkout of a plain folder — is registered as the head of a
 * list, so closing one takes that list with it, exactly as closing a missing checkout does.
 */
async function closeWorkdir(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  if (!checkout) return;
  const scope = checkout.isPrimary ? " and its checkout list" : "";
  const question = `Remove “${checkout.path}”${scope} from Marvis? No files will be deleted, and opening the folder again brings it back.`;
  if (!window.confirm(question)) return;
  try {
    applyWorkspace(await persistCheckoutClose(checkoutId));
  } catch (cause) {
    reportCause(cause);
  }
}

/**
 * The one question the app has open, and what answering "yes" does.
 *
 * The action is held rather than run, because the question is the point: archiving a
 * worktree and putting a set of them back are both things a misclick would undo a
 * stranger's afternoon over, and neither is destructive enough to deserve a dialog of
 * its own the way removing a worktree from disk does.
 */
const pendingConfirm = ref<{
  title: string;
  message: string;
  confirmLabel: string;
  run(): Promise<void>;
} | null>(null);

function askConfirm(question: NonNullable<typeof pendingConfirm.value>) {
  pendingConfirm.value = question;
}

async function answerConfirm(confirmed: boolean) {
  const question = pendingConfirm.value;
  if (!question) return;
  if (!confirmed) {
    pendingConfirm.value = null;
    return;
  }
  try {
    await question.run();
    pendingConfirm.value = null;
  } catch (cause) {
    // The question stays open on a failure, so the answer is not lost to a toast the user
    // has to read while the dialog is still covering the panel.
    reportCause(cause);
  }
}

/**
 * Takes a worktree off the panel and keeps it, so the repo root can put it back.
 *
 * Nothing on disk moves: the branch, its commits and its files stay exactly where they
 * are. That is what the confirmation has to say, because the row cannot — an icon with
 * a box around it reads as "delete" to anyone who has not read this comment.
 */
function archiveWorktree(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  if (!checkout) return;
  askConfirm({
    title: "Archive worktree",
    message: `“${workdirTitle(checkout)}” leaves the sidebar. No files will be deleted, and you can bring it back from the repo row.`,
    confirmLabel: "Archive",
    run: async () => {
      applyWorkspace(await persistCheckoutArchive(checkoutId));
    },
  });
}

/**
 * Puts every worktree this repository archived back on the panel, in one action.
 *
 * It is one action because they were archived one at a time and the panel has room for
 * all of them; walking a set of worktrees back individually is the work the archive was
 * meant to end.
 */
function restoreArchivedWorktrees(repoId: string) {
  const repo = workspace.value.repos.find((item) => item.id === repoId);
  const archived = (workspace.value.archivedWorktrees ?? []).filter((item) => item.repoId === repoId);
  if (!repo || !archived.length) return;
  const plural = archived.length === 1 ? "worktree" : "worktrees";
  askConfirm({
    title: `Restore archived ${plural}`,
    message: `${archived.length} archived ${plural} return to the sidebar. Nothing on disk changes.`,
    confirmLabel: "Restore",
    run: async () => {
      applyWorkspace(await persistArchivedRestore(repoId));
    },
  });
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
  const destination = reviewTarget.value;
  const status = gitSnapshot.status;
  const localTime = localReviewTimestamp();
  const markdown = buildReviewMarkdown(notes, {
    date: localTime.date,
    branch: status?.branch,
    defaultBranch: status?.defaultBranch,
  });
  sendingReview.value = true;
  try {
    if (destination === "markdown") {
      const path = await exportReviewMarkdown(localTime.date, localTime.timestamp, markdown);
      showView(checkout.id, { kind: "document", path, mode: "view", origin: "review" });
      return;
    }
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

async function selectReviewTarget(target: ReviewTarget) {
  const checkoutId = activeCheckout.value?.id;
  if (!checkoutId) return;
  reviewTargetLoadGeneration += 1;
  reviewTarget.value = target;
  try {
    await saveReviewTarget(checkoutId, target);
  } catch (cause) {
    reportCause(cause);
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
  get target() {
    return reviewTarget.value;
  },
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
  selectReviewTarget,
  send: sendReviewToAgent,
};
provide(REVIEW_SENDER, reviewSender);

function updateSessionStatus(sessionId: string, status: TerminalSessionStatus | null) {
  if (status) sessionRuntimeStatuses.value[sessionId] = status;
  else delete sessionRuntimeStatuses.value[sessionId];
}

async function closeTerminalSession(sessionId: string) {
  await mainPane.value?.requestClose(sessionId);
}

/**
 * Names a terminal session.
 *
 * The database decides the name, not this call: it trims, refuses an empty or over-long one
 * and only then writes, so a rejected rename leaves the row showing the name it already had
 * rather than one the backend would not keep.
 */
async function renameTerminalSession(sessionId: string, name: string) {
  const checkoutId = workspace.value.activeCheckoutId;
  if (!checkoutId) return;
  try {
    applyWorkspace(await renameTerminal(checkoutId, sessionId, name));
  } catch (cause) {
    reportCause(cause);
  }
}

function applyWorkspace(next: WorkspaceState) {
  updateWorkspace(next);
}

// A worktree added by something other than this app — an agent running `git worktree add`, a
// script — belongs in the panel the same way one added here does, and the only reading that can
// put it there is the one a launch takes.
useWorktreeSync(() => workspace.value, applyWorkspace, reportCause);

function reportWarning(message: string) {
  pushToast(message, "info");
}
</script>

<template>
  <div
    v-if="appLayoutReady"
    ref="appShell"
    class="app-shell relative flex h-full min-w-[900px] flex-col"
    :style="{ '--inspector-width': `${appLayout.inspectorWidth}px` }"
  >
    <header class="window-header flex h-8 shrink-0 items-center gap-4 border-b pl-[78px] pr-4">
      <!-- The mockup packs the crumbs against the left, with the empty space and the gear at the
           far end. Dragging lives on that empty space, so the crumbs keep their place instead of
           being pushed to the opposite edge. The nav grows into the room the mockup gives it and
           only truncates when it runs out, rather than capping each crumb. -->
      <!-- The crumb line is chrome that names where you are, not text to copy: a drag across it
           was painting a selection over the whole header. -->
      <nav
        ref="lineEl"
        aria-label="Repository location"
        class="window-breadcrumb flex h-full min-w-0 max-w-[70%] shrink select-none items-center gap-1.5"
      >
        <template v-if="activeCheckout">
          <!-- Each crumb is text that opens the list of the level it names, and nothing looks
               like a button until it is pointed at: the workdir, its branch or worktree, and
               the item open inside it. They share one open name, so opening one closes the
               others instead of stacking them. -->
          <TitlebarMenu
            testid="repo-crumb"
            crumb="workdir"
            :label="workdirLabel"
            :sections="workdirMenu"
            :open="openCrumb === 'workdir'"
            search-placeholder="Search workdirs…"
            @update:open="setCrumbOpen('workdir', $event)"
          />
          <template v-if="activeRepo?.kind === 'git'">
            <span aria-hidden="true" class="text-(--marvis-text-faint)">/</span>
            <GitForkIcon class="icon-xs shrink-0" aria-hidden="true" />
            <TitlebarMenu
              testid="worktree-crumb"
              crumb="branch"
              :label="activeCheckout.branch || 'Detached'"
              :sections="worktreeMenu"
              :open="openCrumb === 'worktree'"
              search-placeholder="Search worktrees…"
              @update:open="setCrumbOpen('worktree', $event)"
            />
          </template>
          <!-- The last crumb names the view that is open, and a workdir with no terminal has
               none to name. A file or a change set is picked from the details panel, so it is
               plain text there: a session menu over a diff would name what is not on screen. -->
          <template v-if="hasItemCrumb">
            <span aria-hidden="true" class="text-(--marvis-text-faint)">/</span>
            <TitlebarMenu
              v-if="activeMainView.kind === 'terminal'"
              testid="item-crumb"
              crumb="item"
              align="end"
              :label="activeViewLabel"
              :sections="terminalMenu"
              :open="openCrumb === 'terminal'"
              @update:open="setCrumbOpen('terminal', $event)"
            />
            <span
              v-else
              ref="pathCrumbEl"
              data-testid="item-crumb"
              class="crumb-item min-w-0 truncate"
              :title="activeViewLabel"
            >
              <template v-for="(step, index) in drawnSteps" :key="`${index}-${step}`">
                <span v-if="index > 0" aria-hidden="true" class="crumb-sep">/</span>{{ step }}
              </template>
            </span>
            <!-- The path at the width it wants. It sits beside the crumb rather than inside it
                 so it never joins the text the crumb reads as, and it is always mounted so the
                 crumb can be weighed against the room the line has left. -->
            <span
              v-if="activeMainView.kind !== 'terminal' && pathSteps.length > 2"
              ref="pathProbeEl"
              data-testid="path-probe"
              aria-hidden="true"
              class="path-probe"
              >{{ activeViewLabel }}</span
            >
          </template>
        </template>
      </nav>
      <!-- Native dragging and double-click zoom live on the empty space the mockup leaves at
           the far end, so they cannot swallow a click on a crumb or the gear. -->
      <div data-tauri-drag-region aria-hidden="true" class="h-full min-w-8 flex-1" @dblclick="zoomFromTitlebar" />
      <button
        type="button"
        :aria-label="appLayout.mode === 'split' ? 'Switch to focus layout' : 'Switch to split layout'"
        :aria-pressed="appLayout.mode === 'split'"
        data-testid="layout-toggle"
        class="icon-button shrink-0 text-(--marvis-text-secondary) hover:text-(--marvis-text)"
        @click="toggleLayoutMode"
      >
        <Columns2Icon v-if="appLayout.mode === 'split'" class="icon-xs" aria-hidden="true" />
        <Columns3Icon v-else class="icon-xs" aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label="Settings"
        data-testid="settings-button"
        class="icon-button shrink-0 text-(--marvis-text-secondary) hover:text-(--marvis-text)"
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
          :agent="agent.headline"
          :is-opening="isOpening"
          :archived-worktrees="workspace.archivedWorktrees"
          @open-folder="chooseFolder"
          @select-checkout="activateCheckoutTerminal"
          @select-session="activateTerminalSession"
          @create-worktree="openWorktreeDialog('create', $event)"
          @new-terminal="requestShell"
          @remove-worktree="openWorktreeDialog('remove', $event)"
          @close-workdir="closeWorkdir"
          @close-missing="closeMissingCheckout"
          @archive-worktree="archiveWorktree"
          @restore-archived="restoreArchivedWorktrees"
          @close-session="closeTerminalSession"
          @rename-session="renameTerminalSession"
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
      <SplitterPanel
        id="main-panel"
        :min-size="isSplitLayout ? (activeMainView.kind === 'terminal' ? 320 : 585) : 420"
        size-unit="px"
        class="main-column min-h-0 min-w-0 flex-1"
      >
        <MainPane
          ref="mainPane"
          :checkout="activeCheckout"
          :view="activeMainView"
          :split="isSplitLayout"
          :preview-width="appLayout.previewWidth"
          :terminal-scrollbar="appLayout.terminalScrollbar"
          :ready="checkoutUiReady"
          :git-snapshot="gitSnapshot"
          :review="review"
          :active-session-id="workspace.activeSessionId"
          :is-opening="isOpening"
          :shell-request="shellRequest"
          :registered-session-ids="registeredSessionIds"
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
          @resize-preview="resizeAppPreview"
          @open-markdown-link="activeCheckout && openFileDocument({ checkoutId: activeCheckout.id, path: $event })"
        />
      </SplitterPanel>
      <SplitterResizeHandle
        v-show="!inspectorInDrawer"
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
        :default-size="inspectorInDrawer ? 0 : appLayout.inspectorWidth"
        :min-size="INSPECTOR_WIDTH_LIMITS.min"
        :max-size="INSPECTOR_WIDTH_LIMITS.max"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="inspector-splitter-panel relative min-h-0 shrink-0 overflow-visible"
      >
        <!-- The review, the agent list and the send itself belong to the diff (F.5): this panel
             is the file tree and the change set, and nothing more. -->
        <Teleport :to="appShell" :disabled="!isSplitLayout || !appShell">
          <InspectorPane
            :class="{
              'right-inspector-drawer': inspectorInDrawer,
              'split-inspector-drawer': isSplitLayout,
              'split-inspector-closed': isSplitLayout && !splitInspectorOpen,
            }"
            :checkout="checkoutUiReady ? activeCheckout : null"
            :repo="checkoutUiReady ? activeRepo : null"
            :git-snapshot="gitSnapshot"
            :saved-state="activeCheckout ? checkoutUiStates[activeCheckout.id] : null"
            @open-file="openFileDocument"
            @open-change="openChangedDocument"
            @open-all-changes="openAllChanges"
            @update-ui-state="activeCheckout && updateInspectorUiState(activeCheckout.id, $event)"
            @pointerenter="enterSplitDrawer"
            @pointerleave="leaveSplitDrawer"
          />
        </Teleport>
      </SplitterPanel>
    </SplitterGroup>
    <div
      v-if="isSplitLayout"
      aria-hidden="true"
      class="inspector-hover-strip"
      :class="{ 'inspector-hover-strip-open': splitInspectorOpen }"
      @pointerenter="enterSplitStrip"
      @pointerleave="leaveSplitStrip"
    />
    <ConfirmDialog
      :open="!!pendingConfirm"
      :title="pendingConfirm?.title ?? ''"
      :message="pendingConfirm?.message ?? ''"
      :confirm-label="pendingConfirm?.confirmLabel ?? 'Confirm'"
      @confirm="answerConfirm(true)"
      @close="answerConfirm(false)"
    />
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
