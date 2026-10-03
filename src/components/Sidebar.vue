<script setup lang="ts">
/* The rename field is a void element with enough attributes that it cannot fit on one line, and
   prettier and this rule disagree about how a void element closes: the rule would have it end in
   `>`, the formatter rewrites that to `/>`. The formatter owns it, as in the other panes that
   hold a field. */
/* eslint-disable vue/html-self-closing */
import { computed, nextTick, onUnmounted, ref, shallowRef, toRef } from "vue";
import {
  ArchiveRestore as ArchiveRestoreIcon,
  Ellipsis as EllipsisIcon,
  Folder as FolderIcon,
  FolderGit2 as FolderGit2Icon,
  FolderMinus as FolderMinusIcon,
  FolderX as FolderXIcon,
  GitBranch as GitBranchIcon,
  GitBranchPlus as GitBranchPlusIcon,
  House as HouseIcon,
  Plus as PlusIcon,
  SquareTerminal as SquareTerminalIcon,
  X as XIcon,
} from "@lucide/vue";
import type { ArchivedCheckout, Checkout, Repo, Session, TerminalSessionStatus } from "../domain/workspace";
import { sessionTitle, workdirTitle } from "../domain/workspace";
import type { AgentHeadline } from "../presentation/agent-sessions";
import { useDiffStats } from "../presentation/diff-stats";

defineOptions({ name: "FolderSidebar" });

const props = withDefaults(
  defineProps<{
    repos: Repo[];
    activeCheckoutId: string | null;
    activeSessionId: string | null;
    homeCheckoutId?: string | null;
    isOpening: boolean;
    sessionRuntimeStatuses?: Record<string, TerminalSessionStatus>;
    /**
     * The worktrees that were archived, so a repo root can offer its own back.
     *
     * The sidebar draws the list of what is on the panel; this is what is behind it.
     */
    archivedWorktrees?: ArchivedCheckout[];
    /**
     * The agent of the active checkout, which is the only one with a server behind it.
     *
     * It is a property of the checkout, not of any one terminal, so the rows under the active
     * workdir all repeat it; the `title` on the chip says so rather than implying a link that
     * OpenCode does not offer.
     */
    agent?: AgentHeadline | null;
  }>(),
  {
    homeCheckoutId: null,
    sessionRuntimeStatuses: () => ({}),
    archivedWorktrees: () => [],
    agent: null,
  },
);

const emit = defineEmits<{
  openFolder: [];
  selectCheckout: [checkoutId: string, hasChanges: boolean];
  selectSession: [sessionId: string];
  createWorktree: [checkoutId: string];
  newTerminal: [checkoutId: string];
  removeWorktree: [checkoutId: string];
  closeWorkdir: [checkoutId: string];
  closeMissing: [checkoutId: string];
  restoreArchived: [repoId: string];
  closeSession: [sessionId: string];
  renameSession: [sessionId: string, name: string];
  /** Hands a live terminal to another worktree of the same repository. */
  moveSession: [sessionId: string, targetCheckoutId: string];
}>();

/** The one session whose name is being typed, and the text as typed so far. */
const editingId = ref<string | null>(null);
const draftName = ref("");
const renameField = shallowRef<HTMLInputElement | null>(null);

/**
 * A function ref, because the field sits inside a `v-for`: a plain `ref` there would collect
 * every row's element into an array, and there would be nothing to focus.
 */
function captureRenameField(element: unknown) {
  renameField.value = (element as HTMLInputElement | null) ?? null;
}

function startRename(session: Session) {
  editingId.value = session.id;
  draftName.value = session.name;
  // The text the user is replacing is preselected, so typing overwrites the name rather than
  // appending to it, which is what a rename is for.
  void nextTick(() => {
    renameField.value?.focus();
    renameField.value?.select();
  });
}

/** Nothing typed is not a rename: the row keeps the name it had. */
function commitRename(session: Session) {
  const name = draftName.value.trim();
  editingId.value = null;
  if (name && name !== session.name) emit("renameSession", session.id, name);
}

function cancelRename() {
  editingId.value = null;
}

/** The one session whose destination list is open. */
const moveMenuFor = ref<string | null>(null);

/** The one group whose actions menu is open. */
const groupMenuFor = ref<string | null>(null);

function openGroupMenu(groupId: string) {
  moveMenuFor.value = null;
  groupMenuFor.value = groupId;
  // Capture phase, so a press on anything else closes the menu before that thing reacts to it.
  window.addEventListener("pointerdown", onGroupMenuOutside, true);
  window.addEventListener("keydown", onGroupMenuKeydown);
}

function closeGroupMenu() {
  groupMenuFor.value = null;
  window.removeEventListener("pointerdown", onGroupMenuOutside, true);
  window.removeEventListener("keydown", onGroupMenuKeydown);
}

function toggleGroupMenu(groupId: string) {
  if (groupMenuFor.value === groupId) closeGroupMenu();
  else openGroupMenu(groupId);
}

function onGroupMenuOutside(event: PointerEvent) {
  // The button and the list are inside the menu's own world: pressing them is not "outside".
  if ((event.target as HTMLElement | null)?.closest(".group-menu, .group-more")) return;
  closeGroupMenu();
}

function onGroupMenuKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  closeGroupMenu();
}

interface PointerDrag {
  session: Session;
  pointerId: number;
  originX: number;
  originY: number;
  x: number;
  y: number;
  started: boolean;
  source: HTMLElement;
}

/** A press selects as usual; only a deliberate 5px move turns it into a tree drag. */
const DRAG_THRESHOLD = 5;
const AUTO_SCROLL_EDGE = 36;
const pointerDrag = ref<PointerDrag | null>(null);
const dropCheckoutId = ref<string | null>(null);
const sidebarScroll = ref<HTMLElement | null>(null);
let autoScrollFrame: number | undefined;
let suppressedClickSessionId: string | null = null;
let suppressedClickTimer: number | undefined;

/** A session may land on another live worktree of the same repository, never elsewhere. */
function moveDestination(session: Session, target: Checkout): string | null {
  if (target.isMissing || target.id === session.checkoutId) return null;
  const sourceRepo = props.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === session.checkoutId));
  return sourceRepo?.checkouts.some((checkout) => checkout.id === target.id && !checkout.isMissing) ? target.id : null;
}

function selectSession(sessionId: string, event: MouseEvent) {
  if (event.detail > 0 && suppressedClickSessionId === sessionId) {
    suppressedClickSessionId = null;
    if (suppressedClickTimer !== undefined) window.clearTimeout(suppressedClickTimer);
    suppressedClickTimer = undefined;
    return;
  }
  emit("selectSession", sessionId);
}

function startPointerDrag(session: Session, event: PointerEvent) {
  // Mouse only: a touch pointer must retain the normal vertical scroll gesture of the tree.
  if (event.pointerType !== "mouse" || !event.isPrimary || event.button !== 0) return;
  const sourceRepo = props.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === session.checkoutId));
  if (!sourceRepo?.checkouts.some((checkout) => moveDestination(session, checkout))) return;
  const source = event.currentTarget as HTMLElement;
  pointerDrag.value = {
    session,
    pointerId: event.pointerId,
    originX: event.clientX,
    originY: event.clientY,
    x: event.clientX,
    y: event.clientY,
    started: false,
    source,
  };
  moveMenuFor.value = null;
  if (suppressedClickTimer !== undefined) window.clearTimeout(suppressedClickTimer);
  suppressedClickSessionId = null;
  try {
    source.setPointerCapture(event.pointerId);
  } catch {
    // Global listeners below still track the press if a webview declines pointer capture.
  }
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerCancel);
  window.addEventListener("keydown", onDragKeydown);
  window.addEventListener("blur", cancelPointerDrag);
}

function checkoutAtPoint(x: number, y: number): Checkout | null {
  const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-workdir-checkout]");
  const checkoutId = target?.dataset.workdirCheckout;
  return checkoutId
    ? (props.repos.flatMap((repo) => repo.checkouts).find((checkout) => checkout.id === checkoutId) ?? null)
    : null;
}

function updateDropTarget(x: number, y: number) {
  const drag = pointerDrag.value;
  const target = checkoutAtPoint(x, y);
  dropCheckoutId.value = drag?.started && target ? moveDestination(drag.session, target) : null;
}

function onPointerMove(event: PointerEvent) {
  const drag = pointerDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  drag.x = event.clientX;
  drag.y = event.clientY;
  if (!drag.started && Math.hypot(event.clientX - drag.originX, event.clientY - drag.originY) >= DRAG_THRESHOLD) {
    drag.started = true;
    moveMenuFor.value = null;
  }
  if (!drag.started) return;
  event.preventDefault();
  updateDropTarget(drag.x, drag.y);
  scheduleAutoScroll();
}

function suppressNextClick(sessionId: string) {
  if (suppressedClickTimer !== undefined) window.clearTimeout(suppressedClickTimer);
  suppressedClickSessionId = sessionId;
  suppressedClickTimer = window.setTimeout(() => {
    suppressedClickSessionId = null;
    suppressedClickTimer = undefined;
  }, 0);
}

function onPointerUp(event: PointerEvent) {
  const drag = pointerDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  if (drag.started) updateDropTarget(event.clientX, event.clientY);
  const destination = dropCheckoutId.value;
  const sessionId = drag.session.id;
  const shouldMove = drag.started && destination !== null;
  finishPointerDrag();
  if (drag.started) suppressNextClick(sessionId);
  if (shouldMove && destination) emit("moveSession", sessionId, destination);
}

function onDragKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  cancelPointerDrag();
}

function onPointerCancel(event: PointerEvent) {
  if (pointerDrag.value?.pointerId === event.pointerId) cancelPointerDrag();
}

function cancelPointerDrag() {
  if (pointerDrag.value?.started) suppressNextClick(pointerDrag.value.session.id);
  finishPointerDrag();
}

function finishPointerDrag() {
  const drag = pointerDrag.value;
  pointerDrag.value = null;
  dropCheckoutId.value = null;
  if (autoScrollFrame !== undefined) window.cancelAnimationFrame(autoScrollFrame);
  autoScrollFrame = undefined;
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerCancel);
  window.removeEventListener("keydown", onDragKeydown);
  window.removeEventListener("blur", cancelPointerDrag);
  try {
    if (drag && drag.source.hasPointerCapture(drag.pointerId)) drag.source.releasePointerCapture(drag.pointerId);
  } catch {
    // A pointer can lose capture between its last event and cleanup.
  }
}

function onLostPointerCapture(event: PointerEvent) {
  if (pointerDrag.value?.pointerId === event.pointerId) cancelPointerDrag();
}

function scheduleAutoScroll() {
  if (autoScrollFrame === undefined) autoScrollFrame = window.requestAnimationFrame(autoScroll);
}

function autoScroll() {
  autoScrollFrame = undefined;
  const drag = pointerDrag.value;
  const element = sidebarScroll.value;
  if (!drag?.started || !element) return;
  const bounds = element.getBoundingClientRect();
  const distance =
    drag.y < bounds.top + AUTO_SCROLL_EDGE
      ? drag.y - (bounds.top + AUTO_SCROLL_EDGE)
      : drag.y > bounds.bottom - AUTO_SCROLL_EDGE
        ? drag.y - (bounds.bottom - AUTO_SCROLL_EDGE)
        : 0;
  if (!distance) return;
  const delta = Math.sign(distance) * Math.min(14, Math.max(2, Math.abs(distance) / 2));
  const previous = element.scrollTop;
  element.scrollTop += delta;
  if (element.scrollTop === previous) return;
  updateDropTarget(drag.x, drag.y);
  scheduleAutoScroll();
}

onUnmounted(() => {
  finishPointerDrag();
  closeGroupMenu();
  if (suppressedClickTimer !== undefined) window.clearTimeout(suppressedClickTimer);
});

/** Opens the list without a button: the context-menu key, or the same gesture with a pointer. */
function openMoveMenu(sessionId: string) {
  moveMenuFor.value = moveMenuFor.value === sessionId ? null : sessionId;
}

/** The list closes on the move itself, so a rejected move can be aimed again without a second click. */
function chooseDestination(sessionId: string, targetCheckoutId: string) {
  moveMenuFor.value = null;
  emit("moveSession", sessionId, targetCheckoutId);
}

/** The one program in front of a shell that is also an agent, and so owns the row's agent line. */
const AGENT_APP = "opencode";

/** The icon roles a workdir row can ask for, resolved to a real lucide component. */
const icons = {
  /** A repo root: a folder that is also a Git directory. */
  git: FolderGit2Icon,
  /** A worktree is a branch checked out in a directory of its own, not a fork of anything. */
  worktree: GitBranchIcon,
  folder: FolderIcon,
  home: HouseIcon,
  /** The directory is gone: a folder with a cross, painted in the disabled colour by the row. */
  missing: FolderXIcon,
  terminal: SquareTerminalIcon,
};

type IconKind = keyof typeof icons;

/**
 * What the terminal is doing, as the row's left bar paints it: blue while it runs, red when it
 * ended badly, and no bar at all when it is simply idle or finished cleanly.
 */
type SessionTone = "running" | "error" | "idle";

interface WorkdirItem {
  session: Session;
  /**
   * The worktrees this terminal can be moved to: the others in the same repository.
   *
   * A terminal's session belongs to one worktree, and moving it to a worktree of another
   * repository would mean handing a shell to a Git directory it has nothing to do with — so the
   * destinations are the siblings, and a row with none offers no action.
   */
  destinations: { id: string; label: string; title: string }[];
  active: boolean;
  exited: boolean;
  /** Drives the colour of the row's left bar, independently of whether the row is selected. */
  tone: SessionTone;
  /** The program in front of the shell, when one is: `opencode`, `nvim`. */
  app?: string;
  /** What the row is called on screen: the program in front, or the shell it was opened as. */
  title: string;
  /** The agent this terminal runs, and only when it is the one running it. */
  agent: AgentHeadline | null;
}

interface Workdir {
  checkout: Checkout;
  title: string;
  /** The name cut for drawing: see `branchLabel`. Empty `head` means the name is drawn whole. */
  label: { head: string; tail: string };
  /** The checkout's own line counts, absent when Git has none to show. */
  additions?: number;
  deletions?: number;
  /** What is wrong with this workdir, if anything (E.4). */
  error?: string;
  kind: IconKind;
  /**
   * Worktrees can be removed; repo roots cannot. Everything that acts on the repo as a whole
   * (add a worktree, restore archived ones, take it off the panel) lives in the group header,
   * so a row only ever carries the action that belongs to itself.
   */
  worktree: boolean;
  /**
   * The directory is gone, so the row keeps only its reason for existing and its one way out
   * (closing it). Nothing behind it can be selected, run or created.
   */
  missing: boolean;
  home: boolean;
  active: boolean;
  items: WorkdirItem[];
}

interface Group {
  id: string;
  label: string;
  /** Where the repo lives, cut for drawing; the full path is the tooltip. */
  path: string;
  shortPath: string;
  /** The repo root, which is the checkout the header's actions are addressed to. */
  root: Checkout | null;
  git: boolean;
  home: boolean;
  missing: boolean;
  /** How many worktrees this repo archived. */
  archived: number;
  /** What the header menu offers. A group with none of these has no menu at all. */
  canAdd: boolean;
  canRestore: boolean;
  canClose: boolean;
  hasMenu: boolean;
  workdirs: Workdir[];
}

// The sidebar names every checkout at once, so it asks for the totals of all of them in one
// call rather than per row. The active checkout is named too, so the Changes tab of the
// inspector shares the same refresh instead of running a second listener.
const diffStats = useDiffStats(toRef(props, "repos"), toRef(props, "activeCheckoutId"));

/** How many worktrees each repo archived, counted once so no row walks the list itself. */
const archivedByRepo = computed(() => {
  const counts = new Map<string, number>();
  for (const archived of props.archivedWorktrees) {
    counts.set(archived.repoId, (counts.get(archived.repoId) ?? 0) + 1);
  }
  return counts;
});

const groups = computed<Group[]>(() =>
  props.repos.map((repo) => {
    const root = repo.checkouts.find((checkout) => checkout.isPrimary) ?? repo.checkouts[0] ?? null;
    const git = repo.kind === "git";
    const missing = root?.isMissing ?? false;
    const home = root !== null && root.id === props.homeCheckoutId;
    const archived = archivedByRepo.value.get(repo.id) ?? 0;
    const canAdd = git && !missing && root !== null;
    const canRestore = git && !missing && root !== null && archived > 0;
    const canClose = !home && root !== null;
    return {
      id: repo.id,
      label: repo.name,
      path: root?.path ?? "",
      shortPath: shortPath(root?.path ?? ""),
      root,
      git,
      home,
      missing,
      archived,
      canAdd,
      canRestore,
      canClose,
      hasMenu: canAdd || canRestore || canClose,
      workdirs: repo.checkouts.map((checkout) => toWorkdir(repo, checkout)),
    };
  }),
);

/**
 * A path small enough to sit under a header: the last two segments, with a leading ellipsis when
 * something was dropped. CSS cannot truncate the middle of a string, and the end of a path is
 * the half that says where it is.
 */
function shortPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length <= 2 ? path : `…/${parts.slice(-2).join("/")}`;
}

/** Every header action closes the menu first, so the list never outlives the thing it chose. */
function addWorktree(group: Group) {
  closeGroupMenu();
  if (group.root) emit("createWorktree", group.root.id);
}

function restoreArchived(group: Group) {
  closeGroupMenu();
  if (group.root) emit("restoreArchived", group.root.repoId);
}

/** A missing root is closed through its own event, since there is nothing on disk to close. */
function closeGroup(group: Group) {
  closeGroupMenu();
  if (!group.root) return;
  // Two literal calls rather than one with a computed event name: `emit` is overloaded per
  // event, and a union of names matches none of the overloads.
  if (group.missing) emit("closeMissing", group.root.id);
  else emit("closeWorkdir", group.root.id);
}

/**
 * Whether the row is advertising changes, which is what makes clicking it open the full diff.
 * The counts are the same ones the row paints, so the click and the numbers cannot disagree.
 */
function hasChanges(workdir: Workdir): boolean {
  return Boolean(workdir.additions || workdir.deletions);
}

/**
 * A checkout's name, cut to what identifies it. A ticket-based branch such as
 * `bug/13133933180-fix-login` carries two things the row has no room for at once: the kind
 * (`bug/`) and the ticket number, which is the same noise on every row. The kind stays, the
 * number becomes an ellipsis, and the slug that follows is what the row is really called.
 *
 * The head ends right after the `/` that precedes the ticket segment; the tail begins at the `-`
 * that closes the digits. A name with no ticket in it, like `main` or `release/1.2.0`, has
 * nothing to cut and is drawn whole. The full name is always in the row's tooltip.
 */
function branchLabel(title: string): { head: string; tail: string } {
  const segment = title.slice(title.lastIndexOf("/") + 1);
  const ticket = /^(.*?\d)-(.+)$/.exec(segment);
  if (!ticket) return { head: "", tail: title };
  return { head: title.slice(0, title.length - segment.length) + "…", tail: "-" + ticket[2] };
}

/** The row's name at full length, which the row cannot fit, and where the checkout lives. */
function workdirTooltip(checkout: Checkout): string {
  return checkout.branch ? `${checkout.branch} — ${checkout.path}` : checkout.path;
}

function toWorkdir(repo: Repo, checkout: Checkout): Workdir {
  const isGit = repo.kind === "git";
  const counts = isGit ? diffStats.checkoutTotals[checkout.id] : undefined;
  const title = workdirTitle(repo, checkout);
  return {
    checkout,
    // Git roots use their branch (or "Base" when detached); plain workdirs use the repo name.
    title,
    label: branchLabel(title),
    additions: counts?.additions || undefined,
    deletions: counts?.deletions || undefined,
    // The one failure that belongs to a single workdir (E.4): it names the checkout whose
    // directory is gone, so it belongs in that row and not in a toast about the window.
    error: checkout.isMissing ? "Directory missing" : undefined,
    kind: checkout.isMissing
      ? "missing"
      : checkout.id === props.homeCheckoutId
        ? "home"
        : !isGit
          ? "folder"
          : checkout.isPrimary
            ? "git"
            : "worktree",
    worktree: isGit && !checkout.isPrimary,
    missing: checkout.isMissing,
    home: checkout.id === props.homeCheckoutId,
    active: checkout.id === props.activeCheckoutId,
    items: checkout.sessions.map((session) => {
      const status = props.sessionRuntimeStatuses[session.id];
      const app = status?.foregroundApp;
      return {
        session,
        destinations: repo.checkouts
          .filter((sibling) => sibling.id !== checkout.id && !sibling.isMissing)
          .map((sibling) => ({
            id: sibling.id,
            label: workdirTitle(repo, sibling),
            title: sibling.path,
          })),
        active: session.id === props.activeSessionId,
        exited: sessionState(session) === "exited",
        tone: sessionTone(session),
        app,
        /**
         * The row's name, by the rule the titlebar crumb reads too: `sessionTitle`. The title is
         * separate from process identity, which still owns agent detection.
         */
        title: sessionTitle(session, status),
        /**
         * The agent this terminal is running, and only that: the checkout's agent belongs to a
         * row whose foreground process is the agent, and to no other row in the workdir.
         */
        agent: app === AGENT_APP && checkout.id === props.activeCheckoutId ? (props.agent ?? null) : null,
      };
    }),
  };
}

function sessionState(session: Session) {
  return props.sessionRuntimeStatuses[session.id]?.state ?? (session.status === "active" ? "running" : "exited");
}

/**
 * Running is blue, a shell that ended with a non-zero code is red, anything else is quiet.
 * `exitCode` is read defensively: if the runtime status does not carry one, an exited terminal
 * is simply idle rather than wrongly accused.
 */
function sessionTone(session: Session): SessionTone {
  if (sessionState(session) !== "exited") return "running";
  const status = props.sessionRuntimeStatuses[session.id] as { exitCode?: number | null } | undefined;
  return status?.exitCode ? "error" : "idle";
}

/** A session that is working, stuck or failed stays on screen through a hover. */
function rowPinnedForAttention(item: { agent: AgentHeadline | null }): boolean {
  return item.agent !== null && item.agent.attention !== "none" && item.agent.attention !== undefined;
}

/** Says whose agent it is, which the row cannot know on its own: it is the checkout's. */
const agentTitle = computed(() =>
  props.agent ? `OpenCode agent: ${props.agent.label}` : "No OpenCode agent in this workdir",
);
</script>

<template>
  <aside
    class="app-sidebar flex h-full min-h-0 flex-col border-r text-sm"
    :class="{ 'is-terminal-dragging': pointerDrag?.started }"
  >
    <!-- The right padding is the scrollbar's: macOS draws its own overlay scrollbar over the
         content, so a row whose title and counts end at the edge are read through it. -->
    <div ref="sidebarScroll" class="min-h-0 flex-1 overflow-y-auto pr-2">
      <div v-for="group in groups" :key="group.id" class="workdir-group">
        <!-- The header is where the repo as a whole is acted on, and all of it lives in one menu:
             the three dots, or a right click anywhere on the header. Adding a worktree is the only
             frequent one, but a button that sat two pixels from "remove" made the two easy to
             confuse, so the rare and destructive actions are one click further away. -->
        <div
          class="group-heading"
          :class="{ 'menu-open': groupMenuFor === group.id }"
          @contextmenu.prevent="group.hasMenu && openGroupMenu(group.id)"
        >
          <div class="group-heading-text" :title="group.path || group.label">
            <span class="group-name">{{ group.label }}</span>
            <span v-if="group.shortPath" class="group-path">{{ group.shortPath }}</span>
          </div>

          <button
            v-if="group.hasMenu"
            type="button"
            class="workdir-action group-more"
            :aria-label="`Actions for ${group.label}`"
            aria-haspopup="menu"
            :aria-expanded="groupMenuFor === group.id"
            title="More actions"
            @click="toggleGroupMenu(group.id)"
          >
            <EllipsisIcon class="icon-xs" aria-hidden="true" />
          </button>

          <!-- Hangs below the header rather than pushing the rows, like the move menu. -->
          <ul
            v-if="group.hasMenu && groupMenuFor === group.id"
            class="marvis-menu group-menu"
            role="menu"
            :aria-label="`Actions for ${group.label}`"
          >
            <li v-if="group.canAdd" role="none">
              <button type="button" role="menuitem" class="menu-item select-none text-left" @click="addWorktree(group)">
                <GitBranchPlusIcon class="icon-xs" aria-hidden="true" />
                <span class="menu-item-label">New worktree</span>
              </button>
            </li>
            <!-- Only while there is something to bring back, and the count says how much. -->
            <li v-if="group.canRestore" role="none">
              <button
                type="button"
                role="menuitem"
                class="menu-item select-none text-left"
                :aria-label="`Restore ${group.archived} archived worktree${group.archived === 1 ? '' : 's'}`"
                @click="restoreArchived(group)"
              >
                <ArchiveRestoreIcon class="icon-xs" aria-hidden="true" />
                <span class="menu-item-label">Restore archived worktrees</span>
                <span class="group-menu-count">{{ group.archived }}</span>
              </button>
            </li>
            <!-- Taking the repo off the panel takes its whole list with it. Nothing here deletes a
                 file, and the home directory is the one thing that cannot be closed. -->
            <li v-if="group.canClose" role="none" class="group-menu-danger">
              <button type="button" role="menuitem" class="menu-item select-none text-left" @click="closeGroup(group)">
                <FolderMinusIcon class="icon-xs" aria-hidden="true" />
                <span class="menu-item-label">Remove from panel</span>
              </button>
            </li>
          </ul>
        </div>

        <template v-for="workdir in group.workdirs" :key="workdir.checkout.id">
          <div
            class="workdir-item workdir-parent"
            :data-workdir-checkout="workdir.checkout.id"
            :class="{
              active: workdir.active,
              'has-active': workdir.items.some((item) => item.active),
              'has-action': workdir.worktree && !workdir.home,
              'is-drop-target': dropCheckoutId === workdir.checkout.id,
            }"
          >
            <div class="workdir-row">
              <button
                type="button"
                class="workdir-select"
                :aria-current="workdir.active ? 'page' : undefined"
                :aria-disabled="workdir.missing || undefined"
                :title="workdirTooltip(workdir.checkout)"
                @click="!workdir.missing && emit('selectCheckout', workdir.checkout.id, hasChanges(workdir))"
              >
                <component :is="icons[workdir.kind]" class="workdir-status-icon" aria-hidden="true" />
                <div class="workdir-main">
                  <div class="workdir-title">
                    <!-- The name in two halves, so the ellipsis falls on the tail. The split is a
                         drawing decision and not a change to the name: the two halves sit next to
                         each other with nothing between them, which is what a screen reader, a
                         copy and a test all read. -->
                    <span class="workdir-name">
                      <span v-if="workdir.label.head" class="workdir-name-head">{{ workdir.label.head }}</span>
                      <span class="workdir-name-tail">{{ workdir.label.tail }}</span>
                    </span>
                  </div>
                  <!-- One slot for the row's right-hand text. The error takes it whole: a missing
                       directory has no counts, and a line that mixed a failure with figures
                       would read as two different facts. -->
                  <div class="workdir-meta" :class="{ 'workdir-meta-error': !!workdir.error }">
                    <span v-if="workdir.error">{{ workdir.error }}</span>
                    <template v-else>
                      <span v-if="workdir.additions" class="diff-add">+{{ workdir.additions }}</span>
                      <span v-if="workdir.deletions" class="diff-del">-{{ workdir.deletions }}</span>
                    </template>
                  </div>
                </div>
              </button>

              <!-- A worktree is the only row with an action of its own. A repo root's actions are
                   in the group header above. -->
              <div v-if="workdir.worktree && !workdir.home" class="workdir-actions">
                <!-- A missing directory has nothing to remove from disk, so the row offers
                     the one thing left to do with it: take it off the list. -->
                <button
                  v-if="workdir.missing"
                  type="button"
                  class="workdir-action"
                  :aria-label="`Close missing checkout: ${workdir.title}`"
                  title="Remove from list"
                  @click="emit('closeMissing', workdir.checkout.id)"
                >
                  <XIcon class="icon-xs" aria-hidden="true" />
                </button>
                <!-- Archiving and deleting are two answers to one question, so the row asks it
                     once: the cross opens the dialog that holds both, and nothing is removed,
                     hidden or deleted before the answer comes back. -->
                <button
                  v-else
                  type="button"
                  class="workdir-action"
                  :aria-label="`Remove or archive worktree ${workdir.title}`"
                  title="Remove or archive worktree"
                  @click="emit('removeWorktree', workdir.checkout.id)"
                >
                  <XIcon class="icon-xs" aria-hidden="true" />
                </button>
              </div>
            </div>
          </div>

          <!-- Child items: the same row as the workdir, minus the diff, plus a close.
               "New terminal" is deliberately NOT gated on having terminals open, unlike the
               mockup: picking a workdir no longer opens a terminal by itself, so a workdir
               with nothing running would otherwise offer no way to start one from here. A
               directory that is gone has no live sessions and nothing to run one in.
               The whole list is the rest of the workdir's drop zone: a terminal is dropped on
               the worktree it should belong to, not on whichever of its rows is under it. -->
          <div v-if="!workdir.missing" class="workdir-items" :data-workdir-checkout="workdir.checkout.id">
            <div
              v-for="item in workdir.items"
              :key="item.session.id"
              class="workdir-item workdir-child"
              :class="[
                `tone-${item.tone}`,
                {
                  active: item.active,
                  'is-being-dragged': pointerDrag?.started && pointerDrag.session.id === item.session.id,
                },
              ]"
            >
              <div class="workdir-row">
                <!-- Editing swaps the button for the field, rather than nesting an input inside
                     one: a control inside a control cannot be focused or read on its own. The
                     row keeps its shape because both are laid out the same way. -->
                <div v-if="editingId === item.session.id" class="workdir-select">
                  <component :is="icons.terminal" class="workdir-status-icon" aria-hidden="true" />
                  <div class="workdir-main">
                    <input
                      :ref="captureRenameField"
                      v-model="draftName"
                      class="workdir-rename"
                      type="text"
                      maxlength="60"
                      aria-label="Terminal session name"
                      @keydown.enter.prevent="commitRename(item.session)"
                      @keydown.esc.prevent="cancelRename"
                      @blur="commitRename(item.session)"
                    />
                  </div>
                </div>
                <!-- A terminal moves by mouse after a small pointer threshold; an ordinary click
                     still selects it. The menu remains the keyboard equivalent. -->
                <button
                  v-else
                  type="button"
                  class="workdir-select"
                  :aria-current="item.active ? 'page' : undefined"
                  :aria-label="`Terminal session: ${item.title}`"
                  :aria-haspopup="item.destinations.length ? 'menu' : undefined"
                  :aria-expanded="item.destinations.length ? moveMenuFor === item.session.id : undefined"
                  :title="item.title"
                  @pointerdown="startPointerDrag(item.session, $event)"
                  @lostpointercapture="onLostPointerCapture"
                  @click="selectSession(item.session.id, $event)"
                  @dblclick="startRename(item.session)"
                  @keydown.f2.prevent="startRename(item.session)"
                  @keydown.shift.f10.prevent="item.destinations.length && openMoveMenu(item.session.id)"
                  @contextmenu.prevent="item.destinations.length && openMoveMenu(item.session.id)"
                >
                  <component :is="icons.terminal" class="workdir-status-icon" aria-hidden="true" />
                  <div class="workdir-main">
                    <div class="workdir-title">
                      <span class="workdir-name">{{ item.title }}</span>
                    </div>
                  </div>
                  <!-- The one slot for the row's right-hand text. What is there is context for
                       the row's name and never the thing it is for, so it steps aside for the
                       actions like the counts do — except a session that wants attention, which
                       stays put because that is the news. The agent belongs to the terminal only
                       while OpenCode is the one running in it: a terminal in Neovim is not an
                       agent's terminal, and saying so next to `nvim` would be a claim about a
                       process that is not in front. -->
                  <div
                    v-if="item.agent"
                    class="workdir-meta"
                    :class="{
                      'workdir-meta-error': rowPinnedForAttention(item),
                      'workdir-meta-pinned': rowPinnedForAttention(item),
                    }"
                  >
                    <span class="agent-chip" :title="agentTitle">
                      <span
                        class="agent-dot"
                        :style="{ background: item.agent.color ?? 'var(--marvis-accent)' }"
                        aria-hidden="true"
                      />
                      <span>{{ item.agent.label }}</span>
                      <span v-if="item.agent.attention === 'busy'" class="agent-spinner" aria-hidden="true" />
                    </span>
                  </div>
                </button>

                <div class="workdir-actions">
                  <button
                    type="button"
                    class="workdir-action"
                    :aria-label="`Close terminal session: ${item.title}`"
                    :title="`Close ${item.title}`"
                    @click="emit('closeSession', item.session.id)"
                  >
                    <XIcon class="icon-xs" aria-hidden="true" />
                  </button>
                </div>

                <!-- The destinations as a menu, for the move a drag cannot make. It hangs below the
                     row rather than pushing it, so a list of worktrees never changes the
                     panel it is read from. -->
                <ul
                  v-if="item.destinations.length && moveMenuFor === item.session.id"
                  class="marvis-menu move-menu"
                  role="menu"
                  :aria-label="`Move ${item.title} to`"
                  @keydown.esc.prevent="moveMenuFor = null"
                >
                  <li v-for="destination in item.destinations" :key="destination.id" role="none">
                    <button
                      type="button"
                      role="menuitem"
                      class="menu-item select-none text-left"
                      :title="destination.title"
                      @click="chooseDestination(item.session.id, destination.id)"
                    >
                      <span class="menu-item-label">{{ destination.label }}</span>
                    </button>
                  </li>
                </ul>
              </div>
            </div>

            <div
              v-if="pointerDrag?.started && dropCheckoutId === workdir.checkout.id"
              class="terminal-drop-insertion workdir-child"
              aria-hidden="true"
            >
              <SquareTerminalIcon class="workdir-status-icon" />
              <span>Drop terminal here</span>
            </div>

            <div class="workdir-item workdir-child">
              <div class="workdir-row">
                <button
                  type="button"
                  class="workdir-select new-item"
                  :aria-label="`New terminal for ${workdir.title}`"
                  @click="emit('newTerminal', workdir.checkout.id)"
                >
                  <PlusIcon class="workdir-status-icon" aria-hidden="true" />
                  <div class="workdir-main">
                    <div class="workdir-title">
                      <span class="workdir-name">New terminal</span>
                    </div>
                  </div>
                </button>
              </div>
            </div>
          </div>
        </template>
      </div>

      <p v-if="!groups.length" class="pane-state" role="status">
        Your opened folders will appear here. Use Open directory below to add one.
      </p>
    </div>

    <div class="workdir-item workdir-child shrink-0">
      <div class="workdir-row">
        <button
          type="button"
          class="workdir-select new-item"
          aria-label="Open directory"
          title="Open directory"
          :disabled="isOpening"
          @click="emit('openFolder')"
        >
          <PlusIcon class="workdir-status-icon" aria-hidden="true" />
          <div class="workdir-main">
            <div class="workdir-title">
              <span class="workdir-name">Open directory</span>
            </div>
          </div>
        </button>
      </div>
    </div>
    <Teleport to="body">
      <div
        v-if="pointerDrag?.started"
        class="terminal-drag-ghost"
        aria-hidden="true"
        :style="{ left: pointerDrag.x + 14 + 'px', top: pointerDrag.y + 14 + 'px' }"
      >
        <SquareTerminalIcon class="size-3.5 shrink-0" />
        <span>{{ sessionTitle(pointerDrag.session, sessionRuntimeStatuses[pointerDrag.session.id]) }}</span>
      </div>
    </Teleport>
  </aside>
</template>

<style scoped>
.workdir-group {
  padding: 2px 0 6px;
}

/* Group header: the repo's name at a size and weight that read as a section, its location
   underneath, and one menu button on the right that appears on hover. */
.group-heading {
  position: relative;
  display: flex;
  align-items: center;
  padding: 12px 8px 6px 10px;
}

.group-heading-text {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 1px;
  transition: padding-right 0.12s ease;
}

.group-name {
  overflow: hidden;
  color: var(--marvis-text);
  font-size: 0.875rem;
  font-weight: 600;
  line-height: 1.25;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-path {
  overflow: hidden;
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* One 24px button, so the text only gives up a gutter for it. */
.group-heading:hover .group-heading-text,
.group-heading:focus-within .group-heading-text,
.group-heading.menu-open .group-heading-text {
  padding-right: 30px;
}

.group-more {
  position: absolute;
  top: 50%;
  right: 8px;
  transform: translateY(-50%);
  opacity: 0;
  transition: opacity 0.12s ease;
}

.group-heading:hover .group-more,
.group-heading:focus-within .group-more,
.group-heading.menu-open .group-more {
  opacity: 1;
}

/* The button stays lit while its menu is open, so the menu reads as belonging to it. */
.group-heading.menu-open .group-more {
  background: var(--marvis-control-hover);
  color: var(--marvis-text);
}

/* Hangs below the header, out of flow, and does not fade with a hover: a menu that disappears
   when the pointer leaves the header cannot be read. */
.group-menu {
  position: absolute;
  top: 100%;
  right: 8px;
  z-index: 20;
  min-width: 224px;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  /* The surface is declared here rather than borrowed from `marvis-menu`, which on this panel
     turned out to draw no background: without one, the rows behind the list show through it. */
  background: var(--marvis-control-bg);
  border: 1px solid var(--marvis-control-hover);
  box-shadow: 0 6px 18px rgb(0 0 0 / 40%);
}

.group-menu .menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 6px 10px;
  background: transparent;
  border: none;
  color: var(--marvis-text);
  font: inherit;
  font-size: 0.8125rem;
  cursor: pointer;
}

.group-menu .menu-item:hover,
.group-menu .menu-item:focus-visible {
  background: var(--marvis-control-hover);
}

.group-menu .menu-item .icon-xs {
  flex-shrink: 0;
  color: var(--marvis-text-secondary);
}

.group-menu .menu-item-label {
  flex: 1;
}

.group-menu-count {
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
}

/* The destructive action is set apart by a rule, and is the only place red appears on hover. */
.group-menu li + .group-menu-danger {
  margin-top: 4px;
  padding-top: 4px;
  border-top: 1px solid var(--marvis-control-hover);
}

.group-menu-danger .menu-item:hover {
  color: var(--marvis-danger-fg);
}

/* Row wrapper: keeps the hover/active surface, the select button fills it */
.workdir-item {
  width: 100%;
  display: flex;
  flex-direction: column;
  padding: 5px 8px 5px 10px;
}

/* Select + overlaid actions share a row */
.workdir-row {
  position: relative;
  display: flex;
  align-items: center;
  width: 100%;
}

.workdir-items {
  display: flex;
  flex-direction: column;
}

/* Child items: one level in from the workdir row. Relative, because the destination list hangs
   below the row it belongs to rather than being placed against the panel. */
.workdir-child {
  position: relative;
  padding-left: 24px;
}

.workdir-item:hover {
  background: var(--marvis-control-hover);
}

/* Selection, split by level so that blue marks exactly one thing.
   The terminal is the focus: it takes the selected surface and the accent icon.
   The workdir that holds it is only "where you are": a plain grey, no bar, no accent. */
.workdir-child.active {
  background: var(--marvis-el-selected);
}

.workdir-child.active .workdir-status-icon {
  color: var(--marvis-accent);
}

.workdir-parent.active,
.workdir-parent.has-active {
  background: var(--marvis-control-bg);
}

.workdir-parent.active .workdir-status-icon,
.workdir-parent.has-active .workdir-status-icon {
  color: var(--marvis-text-secondary);
}

/* A workdir selected with no terminal under it is the real focus, so it is the one parent
   that does take the selected surface. */
.workdir-parent.active:not(.has-active) {
  background: var(--marvis-el-selected);
  box-shadow: inset 2px 0 0 var(--marvis-accent);
}

.workdir-parent.active:not(.has-active) .workdir-status-icon {
  color: var(--marvis-accent);
}

/* The left bar of a terminal says what it is doing, whether or not it is selected.
   Running is blue (softened when the row is not the selected one, so a long list of live shells
   does not shout), an error is red at full strength, and an idle terminal has no bar. */
.workdir-child {
  --row-bar: transparent;
  box-shadow: inset 2px 0 0 var(--row-bar);
}

.workdir-child.tone-running {
  --row-bar: color-mix(in srgb, var(--marvis-accent) 45%, transparent);
}

.workdir-child.tone-running.active {
  --row-bar: var(--marvis-accent);
}

.workdir-child.tone-error {
  --row-bar: var(--marvis-danger-fg);
}

.workdir-select {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0;
  background: transparent;
  border: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}

/* A row whose directory is gone stays listed to say so and to be closed. Its label reads as
   unavailable: nothing behind it can be selected, and the single action beside it is the only
   thing the row still does. */
.workdir-select[aria-disabled="true"] {
  color: var(--marvis-text-disabled);
  cursor: not-allowed;
}

.workdir-select[aria-disabled="true"] .workdir-title,
.workdir-select[aria-disabled="true"] .workdir-status-icon {
  color: var(--marvis-text-disabled);
}

.workdir-select.new-item .workdir-name {
  color: var(--marvis-text-dim);
  font-size: 0.75rem;
}

.workdir-select.new-item:hover .workdir-name {
  color: var(--marvis-text);
}

/* Keep the plus in the same 14px layout slot as the terminal icon. Its 1px inset keeps the 12px
   glyph visually light without moving the label two pixels to the left. */
.workdir-select.new-item .workdir-status-icon {
  width: 14px;
  height: 14px;
  padding: 1px;
}

/* Row actions: hover only, never on the selected row. Out of flow, so a resting
   row reserves no gutter for them and the title/diff keep their old alignment.
   Flush to the right edge, so the strip lines up with the diff stats. */
.workdir-actions {
  position: absolute;
  top: 50%;
  right: 0;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.workdir-item:hover .workdir-actions,
.workdir-item:focus-within .workdir-actions {
  opacity: 1;
}

/* Make room under the overlay so a long title ellipsizes instead of running beneath the icon.
   Every row that has an action has exactly one, so the gutter is one 24px icon plus a margin. */
.workdir-item.has-action:hover .workdir-select,
.workdir-child:hover .workdir-select {
  padding-right: 30px;
  transition: padding-right 0.12s ease;
}

.workdir-action {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  background: transparent;
  border: none;
  border-radius: 0;
  color: var(--marvis-text-secondary);
  cursor: pointer;
}

.workdir-action:hover {
  background: var(--marvis-control-hover);
  color: var(--marvis-text);
}

/* The destination list, hanging below the row it belongs to. It is out of flow so a list of
   worktrees does not push the rows after it, and it does not fade with a hover: a menu that
   disappears when the pointer leaves the row cannot be read. */
.move-menu {
  position: absolute;
  top: 26px;
  left: 0;
  z-index: 20;
  min-width: 200px;
}

/* The row a dragged terminal would land in, which is the only thing about a drop that is
   announced before the pointer is let go. */
.workdir-item.is-drop-target {
  background: var(--marvis-control-bg);
  box-shadow: inset 2px 0 0 var(--marvis-accent);
}

.app-sidebar.is-terminal-dragging,
.app-sidebar.is-terminal-dragging * {
  user-select: none !important;
  cursor: grabbing !important;
}

.workdir-child.is-being-dragged {
  opacity: 0.42;
}

.terminal-drop-insertion {
  display: flex;
  min-height: 30px;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  border: 1px dashed var(--marvis-accent);
  color: var(--marvis-accent);
  font-size: 0.6875rem;
}

.terminal-drag-ghost {
  position: fixed;
  z-index: 1000;
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 240px;
  padding: 6px 10px;
  overflow: hidden;
  border: 1px solid var(--marvis-accent);
  background: var(--marvis-control-bg);
  color: var(--marvis-text);
  box-shadow: 0 4px 14px rgb(0 0 0 / 30%);
  pointer-events: none;
  white-space: nowrap;
}

.terminal-drag-ghost span {
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Centered on the title's line box, so it needs no nudging */
.workdir-status-icon {
  width: 14px;
  height: 14px;
  color: var(--marvis-text-faint);
  flex-shrink: 0;
}

.workdir-main {
  min-width: 0;
  flex: 1;
  display: flex;
  gap: 4px;
}

.workdir-title {
  color: var(--marvis-text);
  white-space: nowrap;
  overflow: hidden;
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
}

/* The flex box only clips, so the ellipsis lives on the name itself — and on the name's tail
   rather than on the name, because the tail is the half that says which branch this is. */
.workdir-name {
  display: flex;
  min-width: 0;
  overflow: hidden;
}

/* The head never gives way: it is the kind of branch (`bug/…`, `feat/…`), short and constant, so
   shrinking it only throws away room the tail could have used. The cap is in characters rather
   than in percent of the row, because it is a budget for a prefix: a prefix long enough to miss
   it is cut with an ellipsis instead of taking the row. */
.workdir-name-head {
  flex: 0 0 auto;
  max-width: 18ch;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.workdir-name-tail {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

/* Diff stats share the title's baseline: same line, same center. They step aside
   for the row actions, which is why the box needs no transition of its own. */
.workdir-meta {
  display: flex;
  align-items: center;
  align-self: center;
  gap: 4px;
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
}

/* Hover drops the counts to clear the row actions. An error and a session asking for
   attention stay: the hover gutter already reserves the room, and a row that is being
   hovered at is exactly the row being read. */
.workdir-item:hover .workdir-meta:not(.workdir-meta-error):not(.workdir-meta-pinned) {
  display: none;
}

.workdir-meta-error {
  color: var(--marvis-danger-fg);
}

.workdir-item.has-active .workdir-meta:not(.workdir-meta-error),
.workdir-item.active .workdir-meta:not(.workdir-meta-error) {
  color: var(--marvis-text);
}

/* The rename field takes the row's own type so the text does not jump when it appears. */
.workdir-rename {
  min-width: 0;
  flex: 1;
  padding: 0;
  background: transparent;
  border: none;
  color: var(--marvis-text);
  font: inherit;
}

.workdir-rename:focus-visible {
  outline: 1px solid var(--marvis-control-focus);
  outline-offset: -1px;
}

.agent-chip {
  display: flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
}

/* The agent, in the color OpenCode paints it with. A quiet agent is dimmed like the counts;
   one that is working is not, because that is the reason the reader is looking. */
.agent-chip {
  max-width: 12ch;
  overflow: hidden;
}

.agent-dot {
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: 0;
}

.workdir-meta-pinned .agent-chip {
  color: var(--marvis-text-secondary);
}

.workdir-item:hover .workdir-meta-pinned .agent-chip,
.workdir-item.active .workdir-meta-pinned .agent-chip {
  color: var(--marvis-text);
}

/* A spinner for "working": the turn reports no percentage, so this says only that one is
   open, and the agent's own color is what tells the two apart. */
.agent-spinner {
  width: 7px;
  height: 7px;
  flex-shrink: 0;
  border-radius: 0;
  border: 1px solid currentColor;
  border-top-color: transparent;
  animation: agent-turn 0.7s linear infinite;
}

@keyframes agent-turn {
  to {
    transform: rotate(360deg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .agent-spinner {
    animation: none;
  }
}
</style>
