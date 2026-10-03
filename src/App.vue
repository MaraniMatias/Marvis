<script setup lang="ts">
/* eslint-disable vue/html-closing-bracket-newline, vue/html-indent */
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, nextTick, onMounted, onUnmounted, provide, ref, watch } from "vue";
import { SplitterGroup, SplitterPanel, SplitterResizeHandle } from "reka-ui";
import {
  Columns2 as Columns2Icon,
  Columns3 as Columns3Icon,
  Copy as CopyIcon,
  Minus as MinusIcon,
  Settings as SettingsIcon,
  Square as SquareIcon,
  X as XIcon,
} from "@lucide/vue";
import type { Checkout, Repo } from "./domain/workspace";
import { sessionTitle } from "./domain/workspace";
import { mainViewFromState, mainViewLabel, mainViewToState, resolveMainView } from "./domain/main-document";
import type { DocumentMode, MainView } from "./domain/main-document";
import type { TitlebarMenuItem, TitlebarMenuSection } from "./domain/titlebar-menu";
import InspectorPane from "./components/InspectorPane.vue";
import FileIcon from "./components/FileIcon.vue";
import MainPane from "./components/MainPane.vue";
import Sidebar from "./components/Sidebar.vue";
import TitlebarMenu from "./components/TitlebarMenu.vue";
import ToastStack from "./components/ToastStack.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import WorktreeDialog from "./components/WorktreeDialog.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import type { TerminalSessionStatus, WorkspaceState } from "./domain/workspace";
import { workdirTitle } from "./domain/workspace";
import {
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
import { WORKDIR_ICONS } from "./presentation/workdir-icons";
import { theme } from "./presentation/theme";
import type { Theme } from "./presentation/theme";
import { defaultAgentSession } from "./domain/agent";
import { DEFAULT_ZOOM, zoomKeyFor, zoomLabel, zoomStep } from "./domain/zoom";
import type { Zoom, ZoomModifier } from "./domain/zoom";
import { buildReviewMarkdown, localReviewTimestamp } from "./domain/review";
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
import { DEFAULT_SETTINGS, cloneSettings, normalizeSettings, uiFontScale } from "./domain/settings";
import type { AppSettings } from "./domain/settings";
import {
  loadAppLayout,
  loadCheckoutUiState,
  loadSettings,
  saveAppLayout,
  saveCheckoutUiState,
  saveSettings,
} from "./lib/ipc";

const {
  workspace,
  activeCheckout,
  launchCheckoutId,
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
const { push: pushToast, pushCause: reportCause, dismiss: dismissToast } = useToasts();
const appLayout = ref<AppLayoutState>({ ...DEFAULT_APP_LAYOUT });
const appLayoutReady = ref(false);
const appShell = ref<HTMLElement | null>(null);
const checkoutUiStates = ref<Record<string, CheckoutUiState>>({});
const checkoutUiReady = ref(false);
const mainPane = ref<InstanceType<typeof MainPane> | null>(null);
/** ⌘B's state: the navigation and the files and changes panel are one thing to the eye, so they go
 *  together. A window with the main view alone in it is the point of the shortcut. */
const sidePanelsVisible = ref(true);
const sidebarPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const inspectorPanel = ref<{ collapse(): void; expand(): void; resize(size: number): void } | null>(null);
const viewportWidth = ref(window.innerWidth / DEFAULT_ZOOM);
/** The preferences in `~/.marvis/config.yml`, in the shape the file has them in. */
const settings = ref<AppSettings>(cloneSettings(DEFAULT_SETTINGS));
const settingsButton = ref<HTMLButtonElement | null>(null);
const settingsOpen = ref(false);
const settingsSaving = ref(false);
/** The scale the whole window is drawn at, which every measurement below has to agree with. */
const appZoom = computed(() => settings.value.ui.zoom);
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
watch(launchCheckoutId, (checkoutId) => {
  if (checkoutId) void requestShell(checkoutId);
});

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
let unlistenWindowResized: (() => void) | undefined;
/** Linux draws no frame of its own, so the title bar carries the window's controls there. */
const windowDecorated = ref(true);
const windowMaximized = ref(false);
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
/**
 * The open session under the name the sidebar row gives it.
 *
 * The crumb and the row list the same terminals, so both ask `sessionTitle` for the name: what the
 * PTY last set, the program in front of the shell, or the name it was opened with. A crumb reading
 * the stored name alone said `zsh` over a terminal running something else.
 */
const activeSessionTitle = computed(() => {
  const session = activeSession.value;
  return session ? sessionTitle(session, sessionRuntimeStatuses.value[session.id]) : null;
});
/** What the last crumb names. A file or a change set is not a session, so it is named as itself. */
const activeViewLabel = computed(() => mainViewLabel(activeMainView.value, activeSessionTitle.value));
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
 * path does not get to keep: the workdir, the branch, their glyphs and the separators. The probe
 * is the path at its natural width and it is always mounted, so this weighs two numbers that do
 * not move when the shape on screen does — otherwise an elided path would measure itself as the
 * one that fitted and the line could never come back.
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
    (width, child) =>
      child === crumb || child === probe ? width : width + child.getBoundingClientRect().width / appZoom.value,
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
      items: workspace.value.repos.flatMap((repo) =>
        repo.checkouts.map((checkout) => workdirItem(repo, checkout, `workdir:${checkout.id}`, repo.name)),
      ),
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
      items: repo.checkouts.map((checkout) => workdirItem(repo, checkout, `worktree:${checkout.id}`)),
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

/** The subitems of the active workdir: the terminals it has open, and how to start another. */
const terminalMenu = computed<TitlebarMenuSection[]>(() => {
  const checkout = activeCheckout.value;
  if (!checkout) return [];
  const sessions: TitlebarMenuItem[] = checkout.sessions.map((session) => {
    // The rows are named the way the sidebar names them, so the menu of the open terminal and the
    // row that opened it cannot say two different things about the same session.
    const name = sessionTitle(session, sessionRuntimeStatuses.value[session.id]);
    return {
      id: session.id,
      label: name,
      title: name,
      checked: session.id === workspace.value.activeSessionId,
      run: () => void activateTerminalSession(session.id),
    };
  });
  return [
    { kind: "group", label: "Terminals", items: sessions },
    { kind: "separator" },
    {
      kind: "list",
      items: [{ id: "new-terminal", label: "New terminal", pinned: true, run: () => requestShell(checkout.id) }],
    },
  ];
});

/** What the workdir crumb says: the repo, or the folder when there is no repo to name. */
const workdirLabel = computed(() => activeRepo.value?.name ?? activeCheckout.value?.path.split(/[\\/]/).at(-1) ?? "");

/**
 * The file the last crumb names, when what is open is a file or a change.
 *
 * Its glyph is the one the inspector's file rows give it, so the crumb and the row that opened it
 * cannot say two different things about the same file, and it sits on the step that names the
 * file rather than at the head of the path, which names directories. The whole change set names
 * no file, and a terminal names none either, so the crumb wears no icon in either case.
 */
const itemCrumbFile = computed(() => {
  const view = activeMainView.value;
  if (view.kind === "terminal") return null;
  const name = view.path?.split(/[\\/]/).at(-1);
  return name ? { name, kind: "file" as const } : null;
});

/** One crumb menu at a time: the name it reports becomes the one that is open. */
function setCrumbOpen(name: string, open: boolean) {
  openCrumb.value = open ? name : null;
}

function workdirItem(repo: Repo, checkout: Checkout, id: string, hint?: string): TitlebarMenuItem {
  return {
    id,
    label: workdirTitle(repo, checkout),
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
  // The narrow focus drawer is this same element without the split layout, and a pointer over it
  // there says nothing about a strip that does not exist. A flag left set is a drawer that cannot
  // be closed: the close is refused while either is set, and a hidden drawer fires no leave to
  // clear it — so the drawer opened for one arrangement stays up over the next.
  if (!isSplitLayout.value) return;
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
  if (event.key === "Escape" && splitInspectorOpen.value) {
    closeSplitInspector();
    return;
  }
  if (!appLayoutReady.value || settingsOpen.value) return;
  const direction = zoomKeyFor(event);
  if (direction === undefined) return;
  // The webview is told the scale, and it does nothing with a `0` on its own, but the key is
  // stopped here anyway: a shortcut that also reaches the terminal as input is a shortcut that
  // types into whatever had the focus.
  event.preventDefault();
  setZoom(zoomStep(appZoom.value, direction), event.metaKey ? "Cmd" : "Ctrl");
}

/**
 * ⌘B is the app's and nobody else's, and it takes both side panels with it.
 *
 * This one listens on capture, which is the whole reason it works: by the time a keydown reaches
 * the window on its way up, xterm has already turned it into input, and a shell reading ⌘B as
 * backwards-char is exactly what a sidebar shortcut must not do. Stopping it here means neither
 * the terminal nor the editor ever sees the key, whatever had the focus.
 */
function onSidebarKeydown(event: KeyboardEvent) {
  if (event.altKey || !(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "b") return;
  event.preventDefault();
  event.stopPropagation();
  sidePanelsVisible.value = !sidePanelsVisible.value;
  const show = sidePanelsVisible.value;
  sidebarPanel.value?.[show ? "expand" : "collapse"]();
  // The drawer is the one thing the shortcut can put away without any pointer being over it, and
  // a hidden drawer fires no leave to correct the flags that say one is. Those flags refuse every
  // later close, so the drawer would come back unable to close. Restoring the panels should not
  // find it already open either, which is why this runs in both directions.
  closeSplitInspector();
  // A drawer has no width to give back and no width to take, so what shows it is the state the
  // drawer is bound to. The panel is only the panel's own when it sits beside the main view.
  if (!inspectorInDrawer.value) inspectorPanel.value?.[show ? "expand" : "collapse"]();
}

let zoomToastId: number | undefined;

/**
 * The one line that says where the window is scaled to, replaced rather than stacked.
 *
 * `push` folds away a message identical to the one before it and every step of a held key is a
 * different number, so without taking the last one down first, holding the key leaves the last
 * three percentages on screen as a history of the gesture instead of the current one.
 */
function announceZoom(zoom: Zoom, modifier: ZoomModifier) {
  if (zoomToastId !== undefined) dismissToast(zoomToastId);
  zoomToastId = pushToast(zoomLabel(zoom, modifier), "info");
}

function setZoom(zoom: Zoom, modifier: ZoomModifier) {
  // Already there: the key did something, so there is nothing to say about it.
  if (zoom === appZoom.value) return;
  // The scale responds immediately, but only a saved step is announced as the new preference.
  void applySettings({ ...settings.value, ui: { ...settings.value.ui, zoom } }).then((saved) => {
    if (saved) announceZoom(zoom, modifier);
  });
}

/** The last set the file is known to carry, which is what a rejected write falls back to. */
const savedSettings = ref<AppSettings>(cloneSettings(DEFAULT_SETTINGS));
let settingsWrite: Promise<boolean> = Promise.resolve(true);
let settingsRevision = 0;

/**
 * A preference takes effect at once and is written behind it, with the file as the arbiter.
 *
 * It cannot wait for the write before drawing: the window's scale is a gesture, and a gesture
 * whose next step is computed from a value that has not landed yet is a gesture that only moves
 * once per release. So the setting is adopted immediately and the write follows, queued rather
 * than raced — two presses in a row have to reach the file in the order they were made, or the
 * older set lands last and the next launch opens at the wrong size.
 *
 * A rejected write puts the window back to what the file does carry and says why. That is what
 * makes Cancel in the dialog honest: there is never a state the window is showing and the file
 * does not have, so there is nothing to roll back.
 */
function applySettings(next: AppSettings): Promise<boolean> {
  const normalized = normalizeSettings(next);
  const revision = ++settingsRevision;
  settings.value = normalized;
  settingsWrite = settingsWrite
    .catch(() => false)
    .then(async () => {
      try {
        await saveSettings(normalized);
      } catch (cause) {
        reportCause(cause);
        // A newer preference may already be queued. Only the newest failed write may roll back
        // the optimistic value; an older failure must not erase a later intent.
        if (revision === settingsRevision) settings.value = cloneSettings(savedSettings.value);
        return false;
      }
      savedSettings.value = cloneSettings(normalized);
      return true;
    });
  return settingsWrite;
}

function restoreSettingsFocus() {
  void nextTick(() => settingsButton.value?.focus());
}

function closeSettings() {
  if (settingsSaving.value) return;
  settingsOpen.value = false;
  restoreSettingsFocus();
}

/** The dialog's own Apply: the one place that decides whether the dialog closes. */
async function applyFromDialog(next: AppSettings) {
  settingsSaving.value = true;
  try {
    if (await applySettings(next)) {
      settingsOpen.value = false;
      restoreSettingsFocus();
    }
  } finally {
    settingsSaving.value = false;
  }
}

/**
 * The interface's own sizes are a ratio of one number rather than of the root's font size, because
 * everything here is drawn in pixels: a base font size would move the padding and the icons with
 * the text, and what a person changes in Settings is the text.
 */
const fontScale = computed(() => uiFontScale(settings.value.ui.fontSize));

watch(
  fontScale,
  (scale) => {
    document.documentElement.style.setProperty("--marvis-ui-font-scale", String(scale));
  },
  { immediate: true },
);

/**
 * The palette the window is drawn in, which is the preference resolved: `system` asks the operating
 * system, and the answer changes while the window is open, so this follows it rather than asking
 * once. The attribute is the whole of it for everything a stylesheet paints — the two palettes in
 * `src/marvis.css` are keyed off it — and `theme` carries the answer to the two things CSS cannot
 * reach, the terminal and the diff view.
 *
 * It is asked for and applied while `settings` still holds the defaults, because a first paint in
 * the wrong palette is a frame of a theme the person did not choose.
 */
function applyTheme() {
  const preference = settings.value.ui.theme;
  const resolved: Theme = preference === "system" ? (systemPrefersDark.matches ? "dark" : "light") : preference;
  theme.value = resolved;
  document.documentElement.dataset.theme = resolved;
}

const systemPrefersDark = window.matchMedia("(prefers-color-scheme: dark)");

watch(() => settings.value.ui.theme, applyTheme, { immediate: true });
systemPrefersDark.addEventListener("change", applyTheme);

/**
 * The window is scaled from the root element, so everything inside it is laid out in the space a
 * window of this size has before the scale and painted at the scale after it. Two things follow
 * from that, and both of them are read from here: the viewport a breakpoint is measured against
 * is `zoom` times wider than the space the app is arranging, and a pointer reports where it is in
 * the same widened units.
 */
function applyZoom(zoom: number) {
  document.documentElement.style.setProperty("zoom", String(zoom));
  viewportWidth.value = window.innerWidth / zoom;
}

watch(appZoom, applyZoom, { immediate: true });

/**
 * reka-ui measures its panels with `getBoundingClientRect()` and reports them in the same units,
 * which are the units of the window rather than the units the layout is written in. The two are
 * the same number at 100% and at nothing else, so a width is scaled on its way into the splitter
 * and unscaled on its way back out, and the number in the saved layout keeps meaning what it says.
 */
const toScreen = (px: number) => px * appZoom.value;
const toLayout = (px: number) => px / appZoom.value;

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
  // A close request leaves the webview alive until the flush finishes. If another settings action
  // arrived during that wait, include its newer queue entry before allowing the window to close.
  let revision: number;
  do {
    revision = settingsRevision;
    await settingsWrite;
  } while (revision !== settingsRevision);
}

function onSplitterLayout(sizes: number[]) {
  if (!appLayoutReady.value || sizes.length < 3) return;
  let next = appLayout.value;
  if (sizes[0] > 0) next = resizeLayoutPanel(next, "sidebar", toLayout(sizes[0]));
  if (!inspectorInDrawer.value && sizes[2] > 0) next = resizeLayoutPanel(next, "inspector", toLayout(sizes[2]));
  if (next.sidebarWidth !== appLayout.value.sidebarWidth || next.inspectorWidth !== appLayout.value.inspectorWidth) {
    appLayout.value = next;
  }
}

function resetPanelWidth(panel: "sidebar" | "inspector") {
  const width = panel === "sidebar" ? DEFAULT_APP_LAYOUT.sidebarWidth : DEFAULT_APP_LAYOUT.inspectorWidth;
  (panel === "sidebar" ? sidebarPanel.value : inspectorPanel.value)?.resize(toScreen(width));
  appLayout.value = resizeLayoutPanel(appLayout.value, panel, width);
}

function resizeAppPreview(width: number) {
  appLayout.value = resizeLayoutPanel(appLayout.value, "preview", width);
}

watch(appLayout, () => scheduleAppLayoutSave(), { deep: true });

// A narrow window cannot hold the main panel and the inspector side by side, so the inspector
// floats over it as a drawer. Its width is left alone, to be restored when space returns. A panel
// ⌘B took away is collapsed as well, so a window that grows back does not reserve room for a panel
// nobody can see.
watch(
  [inspectorInDrawer, appLayoutReady],
  async () => {
    if (!appLayoutReady.value) return;
    await nextTick();
    if (inspectorInDrawer.value || !sidePanelsVisible.value) inspectorPanel.value?.collapse();
    else {
      inspectorPanel.value?.expand();
      inspectorPanel.value?.resize(toScreen(appLayout.value.inspectorWidth));
    }
  },
  { immediate: true, flush: "post" },
);

// Either way the pointer is answering a question about an arrangement that no longer exists, and
// the flags that recorded it cannot be corrected by a leave: the strip and the drawer are different
// elements afterwards, so both directions have to start from the drawer closed.
watch(isSplitLayout, () => closeSplitInspector());

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
  window.addEventListener("keydown", onSidebarKeydown, true);
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
    // Whether the window has its own frame decides where the controls live, and it is the
    // only honest answer: a platform that draws one and a platform that does not are told
    // apart by the window rather than by a guess about the user agent.
    windowDecorated.value = await currentWindow.isDecorated();
    if (!windowDecorated.value) {
      windowMaximized.value = await currentWindow.isMaximized();
      unlistenWindowResized = await currentWindow.onResized(() => void readWindowMaximized());
    }
  } catch (cause) {
    showWindowError(cause);
  }
  try {
    appLayout.value = normalizeAppLayout(await loadAppLayout());
  } catch (cause) {
    reportCause(cause);
    appLayout.value = { ...DEFAULT_APP_LAYOUT };
  }
  try {
    settings.value = normalizeSettings(await loadSettings());
    savedSettings.value = cloneSettings(settings.value);
  } catch (cause) {
    // The file is either not there yet or is not something this build can read. Either way the
    // defaults are what the rest of the app draws with, and a broken file stays broken on disk
    // rather than being replaced by them the next time anything is applied.
    reportCause(cause);
  } finally {
    // Do not expose Settings or accept zoom shortcuts until the saved set is known.
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
  unlistenWindowResized?.();
  window.removeEventListener("resize", onViewportResize);
  window.removeEventListener("keydown", onAppKeydown);
  // The capture flag is part of the registration: a listener removed without it is not the one
  // that was added, and this one would outlive the window it belongs to.
  window.removeEventListener("keydown", onSidebarKeydown, true);
  systemPrefersDark.removeEventListener("change", applyTheme);
  unlistenFileActivity?.();
  if (inspectorCloseTimer !== undefined) window.clearTimeout(inspectorCloseTimer);
  if (uiLayoutSaveTimer !== undefined) window.clearTimeout(uiLayoutSaveTimer);
  for (const timer of checkoutUiSaveTimers.values()) window.clearTimeout(timer);
  checkoutUiSaveTimers.clear();
});

function onViewportResize() {
  viewportWidth.value = window.innerWidth / appZoom.value;
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

/**
 * Reads back whether the window is maximized, so the control says what it will do.
 *
 * It is read rather than tracked because the window is not only maximized from here: the
 * desktop maximizes it on its own, and a control that still offered to maximize a maximized
 * window is worse than one that never changed.
 */
async function readWindowMaximized() {
  try {
    windowMaximized.value = await getCurrentWindow().isMaximized();
  } catch (cause) {
    showWindowError(cause);
  }
}

function minimizeWindow() {
  void getCurrentWindow().minimize().catch(showWindowError);
}

function toggleWindowMaximized() {
  void getCurrentWindow().toggleMaximize().then(readWindowMaximized).catch(showWindowError);
}

function closeWindow() {
  void requestWindowClose(getCurrentWindow());
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
 * Hands a live terminal to another worktree of the same repository.
 *
 * The pane owns the live views, so it is the one that can move a terminal without restarting it:
 * this only carries the destination across. The workspace the backend returns already selects the
 * destination worktree, which is what puts the files and the changes panel on it.
 */
async function moveTerminalSession(sessionId: string, targetCheckoutId: string) {
  await mainPane.value?.moveSession(sessionId, targetCheckoutId);
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
    <!-- The padding on the left is where macOS puts its traffic lights. A window that draws no
         frame of its own has none, so the space goes to the crumbs instead. -->
    <header
      class="window-header flex h-8 shrink-0 items-center gap-4 border-b pr-4"
      :class="windowDecorated ? 'pl-[78px]' : 'pl-3'"
    >
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
               others instead of stacking them. The workdir is the one that wears nothing: it is
               the end of the line that is always there, and a name that never changes is one a
               glyph would only repeat. -->
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
            <TitlebarMenu
              testid="worktree-crumb"
              crumb="branch"
              :label="activeCheckout.branch || 'Detached'"
              :icon="WORKDIR_ICONS.worktree"
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
              :icon="WORKDIR_ICONS.terminal"
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
                <span v-if="index > 0" aria-hidden="true" class="crumb-sep">/</span>
                <!-- The last step is the file, so that is the one wearing its icon: the rest of
                     the path is directories, and an icon at the head of the line would name the
                     first of them rather than the thing that is open.

                     The glyph is a box of its own beside the name rather than inside it, because
                     a box holding both would be a flex row, and a flex row takes its baseline
                     from its first item: the step would sit a line below the steps around it. -->
                <span v-if="index === drawnSteps.length - 1 && itemCrumbFile" class="crumb-icon-slot"
                  ><FileIcon class="icon-xs crumb-icon" :name="itemCrumbFile.name" kind="file" /></span
                >{{ step }}
              </template>
            </span>
            <!-- The path at the width it wants, glyph and all: the crumb weighs this probe against
                 the room the line has left, and a path that does not count its own icon would
                 overflow the line before it was told to give anything up. It sits beside the crumb
                 rather than inside it so it never joins the text the crumb reads as, and it is
                 always mounted so the crumb can be weighed against the room the line has left. -->
            <span
              v-if="activeMainView.kind !== 'terminal' && pathSteps.length > 2"
              ref="pathProbeEl"
              data-testid="path-probe"
              aria-hidden="true"
              class="path-probe"
              >{{ activeViewLabel
              }}<span v-if="itemCrumbFile" class="crumb-icon-slot"
                ><FileIcon class="icon-xs crumb-icon" :name="itemCrumbFile.name" kind="file" /></span
            ></span>
          </template>
        </template>
      </nav>
      <!-- Native dragging lives on the empty space the mockup leaves at the far end, so it
           cannot swallow a click on a crumb or the gear. Zooming on a double click is left to
           the window: it reads the double click off the drag region itself, and answering it
           here as well maximized and restored the window in the same gesture. -->
      <div data-tauri-drag-region aria-hidden="true" class="h-full min-w-8 flex-1" />
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
        ref="settingsButton"
        type="button"
        aria-label="Settings"
        data-testid="settings-button"
        class="icon-button shrink-0 text-(--marvis-text-secondary) hover:text-(--marvis-text)"
        @click="settingsOpen = true"
      >
        <SettingsIcon class="icon-xs" aria-hidden="true" />
      </button>
      <!-- The window's own controls, for the platform whose window has no frame to draw them.
           They are past the drag region, so a click on one is a click on the button. -->
      <div v-if="!windowDecorated" class="window-controls -mr-4 flex h-full shrink-0 self-stretch">
        <button
          type="button"
          aria-label="Minimize window"
          data-testid="window-minimize"
          class="window-control"
          @click="minimizeWindow"
        >
          <MinusIcon class="icon-xs" aria-hidden="true" />
        </button>
        <button
          type="button"
          :aria-label="windowMaximized ? 'Restore window' : 'Maximize window'"
          data-testid="window-maximize"
          class="window-control"
          @click="toggleWindowMaximized"
        >
          <!-- Two squares for restore and one for maximize: the same pair every other window
               on the platform uses, which is what makes it readable without a label. -->
          <CopyIcon v-if="windowMaximized" class="icon-xs" aria-hidden="true" />
          <SquareIcon v-else class="icon-xs" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Close window"
          data-testid="window-close"
          class="window-control window-control-close"
          @click="closeWindow"
        >
          <XIcon class="icon-xs" aria-hidden="true" />
        </button>
      </div>
    </header>
    <SplitterGroup direction="horizontal" class="app-splitter flex min-h-0 flex-1" @layout="onSplitterLayout">
      <SplitterPanel
        id="navigation-panel"
        ref="sidebarPanel"
        :default-size="toScreen(appLayout.sidebarWidth)"
        :min-size="toScreen(SIDEBAR_WIDTH_LIMITS.min)"
        :max-size="toScreen(SIDEBAR_WIDTH_LIMITS.max)"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="min-h-0 shrink-0"
      >
        <Sidebar
          :repos="workspace.repos"
          :home-checkout-id="workspace.homeCheckoutId"
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
          @restore-archived="restoreArchivedWorktrees"
          @close-session="closeTerminalSession"
          @rename-session="renameTerminalSession"
          @move-session="moveTerminalSession"
        />
      </SplitterPanel>
      <SplitterResizeHandle
        v-show="sidePanelsVisible"
        id="navigation-resize-handle"
        aria-label="Resize navigation sidebar"
        class="splitter-handle"
        @dblclick.stop="resetPanelWidth('sidebar')"
      >
        <div
          aria-hidden="true"
          class="absolute left-1/2 top-1/2 h-6 w-1 -translate-x-1/2 -translate-y-1/2 bg-(--marvis-text-faint)"
        />
      </SplitterResizeHandle>
      <SplitterPanel
        id="main-panel"
        :min-size="toScreen(isSplitLayout ? (activeMainView.kind === 'terminal' ? 320 : 585) : 420)"
        size-unit="px"
        class="main-column min-h-0 min-w-0 flex-1"
      >
        <MainPane
          ref="mainPane"
          :checkout="activeCheckout"
          :checkouts="allCheckouts"
          :view="activeMainView"
          :split="isSplitLayout"
          :preview-width="appLayout.previewWidth"
          :zoom="appZoom"
          :terminal-settings="settings.terminal"
          :editor-settings="settings.editor"
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
          @open-file="activeCheckout && openFileDocument({ checkoutId: activeCheckout.id, path: $event })"
        />
      </SplitterPanel>
      <SplitterResizeHandle
        v-show="sidePanelsVisible && !inspectorInDrawer"
        id="inspector-resize-handle"
        aria-label="Resize files and changes inspector"
        class="splitter-handle"
        @dblclick.stop="resetPanelWidth('inspector')"
      >
        <div
          aria-hidden="true"
          class="absolute left-1/2 top-1/2 h-6 w-1 -translate-x-1/2 -translate-y-1/2 bg-(--marvis-text-faint)"
        />
      </SplitterResizeHandle>
      <SplitterPanel
        id="inspector-panel"
        ref="inspectorPanel"
        :default-size="inspectorInDrawer ? 0 : toScreen(appLayout.inspectorWidth)"
        :min-size="toScreen(INSPECTOR_WIDTH_LIMITS.min)"
        :max-size="toScreen(INSPECTOR_WIDTH_LIMITS.max)"
        :collapsed-size="0"
        collapsible
        size-unit="px"
        class="inspector-splitter-panel relative min-h-0 shrink-0 overflow-visible"
      >
        <!-- The review, the agent list and the send itself belong to the diff (F.5): this panel
             is the file tree and the change set, and nothing more. -->
        <Teleport :to="appShell" :disabled="!isSplitLayout || !appShell">
          <InspectorPane
            v-show="sidePanelsVisible"
            :class="{
              'right-inspector-drawer': inspectorInDrawer,
              'split-inspector-drawer': isSplitLayout,
              'split-inspector-closed': isSplitLayout && !splitInspectorOpen,
            }"
            :checkout="checkoutUiReady ? activeCheckout : null"
            :repo="checkoutUiReady ? activeRepo : null"
            :git-snapshot="gitSnapshot"
            :saved-state="activeCheckout ? checkoutUiStates[activeCheckout.id] : null"
            :font-scale="fontScale"
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
      v-if="isSplitLayout && sidePanelsVisible"
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
    <SettingsDialog
      :open="settingsOpen"
      :settings="settings"
      :saving="settingsSaving"
      @close="closeSettings"
      @apply="applyFromDialog"
    />
    <ToastStack />
  </div>
  <div v-else class="h-full bg-transparent p-5 text-sm text-zinc-400" role="status">Restoring workspace layout…</div>
</template>
