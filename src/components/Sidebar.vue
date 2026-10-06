<script setup lang="ts">
/* The rename field is a void element with enough attributes that it cannot fit on one line, and
   prettier and this rule disagree about how a void element closes: the rule would have it end in
   `>`, the formatter rewrites that to `/>`. The formatter owns it, as in the other panes that
   hold a field. */
/* eslint-disable vue/html-self-closing */
import type { Component } from "vue";
import { computed, nextTick, onUnmounted, ref, shallowRef, toRef } from "vue";
import {
  ArchiveRestore as ArchiveRestoreIcon,
  ChevronDown as ChevronDownIcon,
  Ellipsis as EllipsisIcon,
  FolderMinus as FolderMinusIcon,
  FolderPlus as FolderPlusIcon,
  GitBranchPlus as GitBranchPlusIcon,
  Plus as PlusIcon,
  SquareTerminal as SquareTerminalIcon,
  X as XIcon,
} from "@lucide/vue";
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from "reka-ui";
import type { ArchivedCheckout, Checkout, Repo, Session, TerminalSessionStatus } from "../domain/workspace";
import { AGENT_APP, agentSessionTitle, sessionRowTitle, workdirIconKind, workdirTitle } from "../domain/workspace";
import type { AgentAttention } from "../domain/agent";
import { agentSessionForTitle } from "../domain/agent";
import type { TerminalAgentRow } from "../presentation/agent-sessions";
import { useDiffStats } from "../presentation/diff-stats";
import { WORKDIR_ICONS } from "../presentation/workdir-icons";

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
     * The order each checkout's terminals are listed in, keyed by checkout id, as the pane saved it.
     *
     * Absent for a checkout whose layout has not been read yet, and the panel falls back to the
     * order it was given rather than inventing one.
     */
    sessionOrder?: Record<string, string[]>;
    /**
     * The OpenCode sessions of each checkout that has terminals, keyed by checkout id.
     *
     * Keyed by checkout rather than handed over as one summary because a row is about one terminal
     * in one worktree: two terminals in different worktrees are different rows, and each reads only
     * its own. OpenCode 2.0.22 cannot report which session a given terminal has open, so this is
     * every session of the worktree and the row matches them by the title its own terminal carried
     * (`agentSessionForTitle`). A row that matches none draws no state at all, which is what keeps
     * a worktree's unrelated sessions off a terminal that is not showing them.
     */
    agentRows?: Record<string, TerminalAgentRow>;
    /**
     * The worktrees that were archived, so a repo root can offer its own back.
     *
     * The sidebar draws the list of what is on the panel; this is what is behind it.
     */
    archivedWorktrees?: ArchivedCheckout[];
  }>(),
  {
    homeCheckoutId: null,
    sessionRuntimeStatuses: () => ({}),
    sessionOrder: () => ({}),
    agentRows: () => ({}),
    archivedWorktrees: () => [],
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
  moveSession: [sessionId: string, targetCheckoutId: string, index: number];
}>();

/** Whether the terminal under the pointer came out of this checkout's list. */
function isDraggedFrom(checkout: Checkout): boolean {
  const drag = pointerDrag.value;
  return Boolean(drag?.started && drag.session.checkoutId === checkout.id);
}

/**
 * The slot this row sits in, counted the way `dropIndexAtPoint` counts.
 *
 * The drop line is drawn before a row, so its condition needs the same number the measurement
 * produced — and the measurement leaves the dragged terminal out. Comparing it against the drawn
 * index instead put the line one row too high, which is worse than an offset: inserting a row there
 * moved every row below it down, so the next measurement of the same still pointer landed on a
 * different row and the line vanished.
 */
function dropSlotFor(checkout: Checkout, itemIndex: number): number {
  if (!isDraggedFrom(checkout)) return itemIndex;
  const draggedIndex = orderedSessions(checkout).findIndex((session) => session.id === pointerDrag.value?.session.id);
  return draggedIndex >= 0 && itemIndex > draggedIndex ? itemIndex - 1 : itemIndex;
}

/**
 * How many slots this list has while the dragged terminal is out of the way.
 *
 * The slot past the last row is drawn separately, and it needs the same count: with the dragged row
 * still counted, "past the end" is one slot too far and the line for the real end never appears.
 */
function dropSlotCount(checkout: Checkout): number {
  return orderedSessions(checkout).length - (isDraggedFrom(checkout) ? 1 : 0);
}

/**
 * Whether the drop line belongs immediately above this row.
 *
 * The whole condition lives here rather than in the template because it says the same thing in two
 * places, and a template that spells it out twice is one that will spell it out slightly differently
 * twice.
 */
function dropLineBefore(checkout: Checkout, itemIndex: number): boolean {
  return (
    Boolean(pointerDrag.value?.started) &&
    dropCheckoutId.value === checkout.id &&
    dropIndex.value === dropSlotFor(checkout, itemIndex)
  );
}

/** And whether it belongs past the last row instead, which is a slot of its own. */
function dropLineAfter(checkout: Checkout): boolean {
  return (
    Boolean(pointerDrag.value?.started) &&
    dropCheckoutId.value === checkout.id &&
    dropIndex.value >= dropSlotCount(checkout)
  );
}

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

/**
 * The one group whose actions menu is open.
 *
 * The menu is reka's, like the titlebar's: it brings the outside press, the Escape, the arrows,
 * the typeahead and the focus back to the button, none of which the panel was giving it. What the
 * panel keeps is which of the groups is open, because that is the one thing reka does not know:
 * there is a menu per group and only one of them is allowed to be.
 */
const groupMenuFor = ref<string | null>(null);

/** A right click anywhere on the header opens the menu the button opens. */
function openGroupMenu(groupId: string) {
  moveMenuFor.value = null;
  groupMenuFor.value = groupId;
}

/** Closing, from the menu itself or from a row that was chosen. reka says when. */
function closeGroupMenu() {
  groupMenuFor.value = null;
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
/** The slot in that checkout's list the dragged terminal would land in. */
const dropIndex = ref(0);
/** The pointer's own y, so a drop above or below a row can be told without re-reading the event. */
const dropPointerY = ref(0);
const sidebarScroll = ref<HTMLElement | null>(null);
let autoScrollFrame: number | undefined;
let suppressedClickSessionId: string | null = null;
let suppressedClickTimer: number | undefined;

/** A session may land on another live worktree of the same repository, never elsewhere. */
function moveDestination(session: Session, target: Checkout, index: number): string | null {
  if (target.isMissing) return null;
  const sourceRepo = props.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === session.checkoutId));
  const known = sourceRepo?.checkouts.some((checkout) => checkout.id === target.id && !checkout.isMissing);
  if (!known) return null;
  // Its own worktree is now a destination too, because a terminal can be reordered inside it. The slot
  // it already sits in is not a destination: dropping a row where it is would report a move that
  // changes nothing.
  if (target.id === session.checkoutId) {
    const current = orderedSessions(target).findIndex((item) => item.id === session.id);
    return current < 0 || current === index ? null : target.id;
  }
  return target.id;
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
  const home = props.repos.flatMap((repo) => repo.checkouts).find((checkout) => checkout.id === session.checkoutId);
  // A drag is worth starting when another worktree can take it, or when its own worktree holds more
  // than one terminal and it can therefore be reordered.
  const elsewhere = props.repos
    .flatMap((repo) => repo.checkouts)
    .some((checkout) => moveDestination(session, checkout, 0));
  if (!home || (!elsewhere && orderedSessions(home).length < 2)) return;
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

/**
 * Where in the target's list the dragged terminal would land: which slot, counted from the top.
 *
 * **The dragged row is not one of the rows counted.** It is drawn at its old position until it is
 * dropped, so a slot counted with it in place would mean one thing for a terminal travelling up and
 * another for the same terminal travelling down — and the difference is a row, which is where the
 * terminal ends up. Measured without it, one number is the same before the drop line is drawn, in the
 * line itself and in the layout that gets saved.
 *
 * It is also measured against the pointer's own position rather than the one the last move carried,
 * which is a frame late and therefore a slot wrong at every edge of a row.
 */
function dropIndexAtPoint(target: HTMLElement, dragging: Session | null): number {
  const rows = [...target.querySelectorAll<HTMLElement>(".workdir-child[data-session-id]")].filter(
    (row) => !row.classList.contains("new-item") && row.dataset.sessionId !== dragging?.id,
  );
  const index = rows.findIndex((row) => {
    const box = row.getBoundingClientRect();
    return dropPointerY.value < box.top + box.height / 2;
  });
  return index < 0 ? rows.length : index;
}

function checkoutAtPoint(x: number, y: number, dragging: Session | null): { checkout: Checkout; index: number } | null {
  const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-workdir-checkout]");
  const checkoutId = target?.dataset.workdirCheckout;
  const checkout = checkoutId
    ? props.repos.flatMap((repo) => repo.checkouts).find((item) => item.id === checkoutId)
    : null;
  return checkout && target ? { checkout, index: dropIndexAtPoint(target, dragging) } : null;
}

function updateDropTarget(x: number, y: number) {
  const drag = pointerDrag.value;
  // The pointer's own y is set before it is measured against anything: measuring first would answer
  // with the previous move's position.
  dropPointerY.value = y;
  const at = checkoutAtPoint(x, y, drag?.started ? drag.session : null);
  dropCheckoutId.value = drag?.started && at ? moveDestination(drag.session, at.checkout, at.index) : null;
  dropIndex.value = dropCheckoutId.value ? (at?.index ?? 0) : 0;
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
  const index = dropIndex.value;
  if (drag.started) suppressNextClick(sessionId);
  if (shouldMove && destination) emit("moveSession", sessionId, destination, index);
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
  if (suppressedClickTimer !== undefined) window.clearTimeout(suppressedClickTimer);
});

/** Opens the list without a button: the context-menu key, or the same gesture with a pointer. */
function openMoveMenu(sessionId: string) {
  moveMenuFor.value = moveMenuFor.value === sessionId ? null : sessionId;
}

/**
 * The list closes on the move itself, so a rejected move can be aimed again without a second click.
 *
 * The menu has no position to give: it names a worktree, and a worktree takes the terminal at the end
 * of its list, which is where the row is appended when nothing else says otherwise.
 */
function chooseDestination(sessionId: string, targetCheckoutId: string) {
  moveMenuFor.value = null;
  const target = props.repos.flatMap((repo) => repo.checkouts).find((checkout) => checkout.id === targetCheckoutId);
  // A menu names a worktree and cannot name a row inside it, so it takes the end of the list: that is
  // the slot below every terminal the worktree already has, and no row of the destination is offered
  // as a position because none was chosen.
  emit("moveSession", sessionId, targetCheckoutId, target ? orderedSessions(target).length : 0);
}

/**
 * The one thing a row's icon can say, and the colour it says it in.
 *
 * The icon *is* the state in this design: there is no corner badge, no chip and no second line, so
 * what the row is doing is drawn by which glyph it wears and which colour that glyph has. Only
 * `working` moves; the rest are states, and a state is noticed without moving.
 */
type RowState = "working" | "waiting" | "failed" | "running" | "idle";

/**
 * What each agent state is called, beside the glyph and the colour it wears.
 *
 * `blocked` is the service reporting a turn stuck on a permission this server version cannot
 * answer, which is the only honest reading of "waiting for you" available here: nothing in the
 * protocol says who is supposed to answer, only that the turn has not moved.
 */
const AGENT_STATE: Record<AgentAttention, { state: RowState; text: string }> = {
  busy: { state: "working", text: "Working" },
  blocked: { state: "waiting", text: "Waiting for your reply" },
  failed: { state: "failed", text: "Last turn failed" },
  none: { state: "idle", text: "Idle" },
};

/**
 * What a row with the agent in front of it says when nothing said which session it has open.
 *
 * OpenCode 2.0.22 offers no route that maps a TUI process to a session, so an agent terminal whose
 * own title named nothing has no session to draw a state from — and "idle" is the one reading the
 * row must not assert: nothing observed a turn to be still, and nothing observed one to be running.
 * So the glyph stays the agent's and goes grey, exactly as an idle agent's does, and the honest
 * difference is spelled out in words beside the name: `sin sesión`, the reference's own sentence,
 * which says a session is missing rather than inventing one.
 */
const NO_AGENT_SESSION = "sin sesión";

interface WorkdirItem {
  session: Session;
  /**
   * The worktrees this terminal can be moved to: the others in the same repository.
   *
   * A terminal's session belongs to one worktree, and moving it to a worktree of another
   * repository would mean handing a shell to a Git directory it has nothing to do with. So the
   * destinations are the siblings, and a row with none offers no action.
   */
  destinations: { id: string; label: string; title: string }[];
  active: boolean;
  /**
   * The row's own name: the agent session's own title where this terminal's title names one, and
   * the program in front of the shell or the session's own name otherwise. See `toWorkdir`, which
   * is where the attribution is decided.
   */
  title: string;
  /**
   * The secondary detail, inline and muted, and the first thing to give way when the row is short.
   *
   * Three answers, all of them a fact rather than a filler:
   *
   * - An identified agent session's mode: `plan`, `coder`. It is the one word the service named for
   *   this terminal's own session, and it sits beside the session's title rather than under it.
   * - `sin sesión`, when OpenCode is in front but nothing identified which session. The row's name is
   *   already `opencode`, so this says the missing thing rather than repeating the present one.
   *
   * A plain terminal has none, and that is the user's call after seeing it: the worktree name beside
   * `zsh` repeated the group row directly above it, three rows under it, spelling the same branch
   * four times in one block. Indentation already says which group a row is in, so repeating it says
   * nothing new and costs the name the width. Two idle shells in one worktree therefore read as two
   * identical rows — which is true, because nothing observed them apart.
   */
  detail?: string;
  /** The glyph the row wears: what it is. */
  icon: Component;
  /** What the glyph says about it, in colour. The state lives here and nowhere else. */
  state: RowState;
  /**
   * How long ago the session this terminal has open was last updated, drawn compactly.
   *
   * Real or nothing: the only clock this app has for a session is the service's own `updatedAt`,
   * so a row with no identified session has no time to draw and draws none. Never a duration
   * measured from when the panel happened to open or from when a row was created.
   */
  elapsed: string | null;
  /**
   * The state in words, for the row's tooltip and its accessible name. Empty on a row that is only
   * a shell, because a shell at a prompt has nothing to report.
   */
  note: string;
}

interface Workdir {
  checkout: Checkout;
  title: string;
  /** The checkout's own line counts, absent when Git has none to show. */
  additions?: number;
  deletions?: number;
  /** What is wrong with this workdir, if anything (E.4). */
  error?: string;
  /** The glyph the row wears: the same one the titlebar's crumb for this checkout wears. */
  icon: Component;
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
  /**
   * The current checkout, which is context and not selection.
   *
   * It is what the app is pointed at, and it is said by neutral ink — see the group glyph rule — never
   * by the accent, because the accent means one thing only and that thing is `selected`. A checkout
   * holding the terminal being read is a place you are in, not a row you have chosen.
   */
  active: boolean;
  /**
   * The one row in the whole panel that is selected: this checkout row, when this checkout is the
   * current one *and* has no terminal of its own selected.
   *
   * That conjunction is what makes the invariant hold without a rule that has to be defended. A
   * terminal row is selected by its own session, so while one of this checkout's terminals is
   * selected the branch row gives the selection up and the terminal wears it alone; when none is, the
   * branch is what is selected and takes it. Either way exactly one row in the panel carries the
   * accent edge, which is the only thing the edge is allowed to mean.
   */
  selected: boolean;
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

/** The row's name at full length, which the row cannot fit, and where the checkout lives. */
function workdirTooltip(checkout: Checkout): string {
  return checkout.branch ? `${checkout.branch} — ${checkout.path}` : checkout.path;
}

/**
 * How long ago a session was last touched, drawn the way a person writes it: `12s`, `2m`, `1h`.
 *
 * The service's own `updatedAt` is the only clock here, so a row whose session was never
 * identified has no number to draw and this is never called for it. Anything a minute old or more
 * is rounded rather than floored, because `59s` and `60s` say the same thing and the shorter one
 * would be a lie about how long.
 */
function elapsedSince(updatedAt: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - updatedAt) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.round(minutes / 60)}h`;
}

/**
 * The terminals of a checkout in the order they are listed, which is not the order the database
 * hands them out.
 *
 * The pane owns that order — it is what gets saved, and it is where a drag puts a terminal — so it
 * publishes it and this reads it. A list drawn in the database's order would show a reorder as
 * nothing at all: persisted, correct, and invisible on the only screen that draws it.
 */
function orderedSessions(checkout: Checkout): Session[] {
  const order = props.sessionOrder?.[checkout.id];
  if (!order) return checkout.sessions;
  const byId = new Map(checkout.sessions.map((session) => [session.id, session] as const));
  const ordered = order.flatMap((id) => (byId.get(id) ? [byId.get(id)!] : []));
  const seen = new Set(ordered.map((session) => session.id));
  return [...ordered, ...checkout.sessions.filter((session) => !seen.has(session.id))];
}

function toWorkdir(repo: Repo, checkout: Checkout): Workdir {
  const isGit = repo.kind === "git";
  const counts = isGit ? diffStats.checkoutTotals[checkout.id] : undefined;
  const title = workdirTitle(repo, checkout);
  return {
    checkout,
    // Git roots use their branch (or "Base" when detached); plain workdirs use the repo name.
    title,
    additions: counts?.additions || undefined,
    deletions: counts?.deletions || undefined,
    // The one failure that belongs to a single workdir (E.4): it names the checkout whose
    // directory is gone, so it belongs in that row and not in a toast about the window.
    error: checkout.isMissing ? "Directory missing" : undefined,
    icon: WORKDIR_ICONS[workdirIconKind(repo, checkout, props.homeCheckoutId)],
    worktree: isGit && !checkout.isPrimary,
    missing: checkout.isMissing,
    home: checkout.id === props.homeCheckoutId,
    active: checkout.id === props.activeCheckoutId,
    // Exactly one row in the panel wears the accent edge. A terminal row is selected by its own
    // session, so a branch that holds the selected terminal gives the selection up; a branch with
    // no selected terminal of its own takes it, and that is the branch being what is open.
    selected:
      checkout.id === props.activeCheckoutId &&
      !checkout.sessions.some((session) => session.id === props.activeSessionId),
    items: orderedSessions(checkout).map((session) => {
      const status = props.sessionRuntimeStatuses[session.id];
      const app = status?.foregroundApp;
      /**
       * The session this terminal is showing, read from this terminal's own worktree and matched by
       * this terminal's own title.
       *
       * Two conditions, both about this row rather than about the worktree, and both refused rather
       * than relaxed: the runtime has to confirm OpenCode is the program in front of this terminal
       * right now (`agentSessionTitle`), and the title that program wrote has to name exactly one
       * session of this worktree (`agentSessionForTitle`). A worktree can hold a checkout's worth of
       * unrelated sessions, so nothing falls back to the newest, the loudest or the only one: two
       * OpenCode terminals in one worktree with no live title are both unidentified rather than one
       * of them wearing the other's name. The match is inference, not a mapping the service
       * confirms — `agentSessionForTitle` says what cannot be told apart here — so an unconfirmed
       * answer draws no state rather than a borrowed one.
       */
      const agentTerminal = app === AGENT_APP;
      const identified = agentTerminal
        ? agentSessionForTitle(props.agentRows[checkout.id]?.sessions ?? [], agentSessionTitle(status))
        : null;
      /**
       * One attention for the whole row, and the session's own answer wins.
       *
       * A session the service reports as running is working whether or not it has an agent to name.
       * `attention` is only carried for a session that HAS an agent, so reading it alone drew a
       * running turn as idle grey: the row said nothing was happening while the service said a turn
       * was open. A session that names no agent and runs no turn is idle, which is a fact rather than
       * a shrug.
       */
      const attention: AgentAttention = identified
        ? (identified.agent?.attention ?? (identified.running ? "busy" : "none"))
        : "none";
      const agent = identified ? AGENT_STATE[attention] : undefined;
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
        /**
         * The row's name, which `sessionRowTitle` writes: the session this terminal is showing when
         * its own title names one, the program in front of the shell otherwise, and the name it was
         * opened with — or renamed to — when nothing is. Nothing about the workdir: that is the
         * muted half beside it, and it is there so two identical shells are not identical rows.
         */
        title: sessionRowTitle(session, status),
        detail: identified
          ? (identified.agent?.label ?? NO_AGENT_SESSION)
          : agentTerminal
            ? NO_AGENT_SESSION
            : undefined,
        /**
         * The glyph and its colour, which between them are the whole of the row's state.
         *
         * - An identified session draws the state the service reported, and only that session's:
         *   the spinner in the accent while it works, the sparkles in the warning colour while it
         *   waits for a reply, the sparkles in the danger colour when its last turn failed.
         * - A plain terminal with something in front of the shell is a process that is up, which is
         *   green and which nothing else in the panel is.
         * - Everything else is idle: the agent's own glyph or a terminal's, both grey. An OpenCode
         *   whose session nobody identified lands here too, which is why it also says `sin sesión` —
         *   a colour cannot say "nothing was observed" without lying about the state.
         */
        icon: agentTerminal
          ? agent?.state === "working"
            ? WORKDIR_ICONS.working
            : WORKDIR_ICONS.agent
          : WORKDIR_ICONS.terminal,
        // An OpenCode whose session nobody identified has no state to draw, so it is idle rather
        // than green: `running` on a row with an agent glyph in front would claim a process that
        // the panel cannot identify, and the word beside it already says what is missing.
        state: agent?.state ?? (agentTerminal ? "idle" : status?.foregroundProcess ? "running" : "idle"),
        // Only a row that identified a session has a clock to read, and that row is the only one
        // that draws a time.
        elapsed: identified ? elapsedSince(identified.updatedAt) : null,
        // The words say the same thing the glyph says, and a session that was identified and is
        // simply quiet says it is idle rather than claiming that nothing could be found.
        note: agent ? agent.text : agentTerminal ? "Session not identified" : "",
      };
    }),
  };
}

/**
 * The row the ghost under the pointer is a copy of, so the name under the cursor is the name the
 * row it came from wears. Read from the same model rather than named a second time.
 */
const draggedItem = computed(
  () =>
    groups.value
      .flatMap((group) => group.workdirs)
      .flatMap((workdir) => workdir.items)
      .find((item) => item.session.id === pointerDrag.value?.session.id) ?? null,
);

/**
 * The groups that are folded away, by checkout id.
 *
 * Local state and nothing else, on purpose. A fold is one person's view of a list they are reading
 * right now, not a preference they set once and expect to still be there tomorrow — and a preference
 * is a field in `config.yml` and a line in the Settings dialog, with its own key on both sides. If it
 * should be remembered, that is a separate decision; it is not smuggled in here as a field nobody
 * declared.
 */
const collapsedGroups = ref(new Set<string>());

function isCollapsed(workdir: Workdir): boolean {
  return collapsedGroups.value.has(workdir.checkout.id);
}

function toggleGroup(workdir: Workdir) {
  const next = new Set(collapsedGroups.value);
  if (!next.delete(workdir.checkout.id)) next.add(workdir.checkout.id);
  collapsedGroups.value = next;
}

/**
 * The repositories folded away, by repo id.
 *
 * Separate from the checkouts' own fold rather than the same set: a repo folded shut has its
 * checkouts hidden, so their chevrons are not on screen to say anything, and a set that held both
 * would let one control answer for the other.
 */
const collapsedRepos = ref(new Set<string>());

function isRepoCollapsed(group: Group): boolean {
  return collapsedRepos.value.has(group.id);
}

function toggleRepo(group: Group) {
  const next = new Set(collapsedRepos.value);
  if (!next.delete(group.id)) next.add(group.id);
  collapsedRepos.value = next;
}

/**
 * What the row announces: its name, its detail, and the state in words.
 *
 * The state is drawn as a colour on a glyph, and a colour is not something a screen reader reads, so
 * it is spelled out here. Both answers come from the same `item.state`, so the drawn colour and the
 * announced word can never disagree.
 */
function rowLabel(item: WorkdirItem): string {
  const said = [`${item.title}${item.detail ? `, ${item.detail}` : ""}`];
  if (item.elapsed) said.push(item.elapsed);
  if (item.note) said.push(item.note);
  return said.join(" — ");
}
</script>

<template>
  <aside
    class="app-sidebar flex h-full min-h-0 flex-col border-r"
    :class="{ 'is-terminal-dragging': pointerDrag?.started }"
  >
    <!-- The right padding is the scrollbar's: macOS draws its own overlay scrollbar over the
         content, so a row whose name and counts end at the edge are read through it. -->
    <div ref="sidebarScroll" class="sidebar-scroll">
      <div
        v-for="group in groups"
        :key="group.id"
        class="workdir-group"
        :class="{ 'is-collapsed': isRepoCollapsed(group) }"
      >
        <!-- The header is where the repo as a whole is acted on, and all of it lives in one menu:
             the three dots, or a right click anywhere on the header. Adding a worktree is the only
             frequent one, but a button that sat two pixels from "remove" made the two easy to
             confuse, so the rare and destructive actions are one click further away.

             The name folds the repository, which is why it is a button and not a `div` with a click
             on it: folding a list has to be reachable by keyboard, and `aria-expanded` is what says
             which way it goes — the header carries no glyph for it, because a folded repository is
             already legible as one: its row and nothing under it. The menu button is a sibling
             rather than a child, because a button inside a button is not a control a person can
             operate. -->
        <div
          class="group-heading"
          :class="{ 'menu-open': groupMenuFor === group.id }"
          @contextmenu.prevent="group.hasMenu && openGroupMenu(group.id)"
        >
          <button
            type="button"
            class="group-heading-text"
            :title="group.path || group.label"
            :aria-expanded="!isRepoCollapsed(group)"
            @click="toggleRepo(group)"
          >
            <span class="group-name">{{ group.label }}</span>
            <span v-if="group.shortPath" class="group-path">{{ group.shortPath }}</span>
          </button>

          <!-- reka's menu, the same one the titlebar crumbs open, so the outside press, the
               Escape, the arrows and the focus back to the button are the library's to get right.
               The panel only says which group is open: there is a menu per group and `groupMenuFor`
               is the one that may be. A right click on the header sets it directly, which is the
               only way in that the trigger is not part of. -->
          <DropdownMenuRoot
            v-if="group.hasMenu"
            :open="groupMenuFor === group.id"
            @update:open="(isOpen) => (isOpen ? openGroupMenu(group.id) : closeGroupMenu())"
          >
            <DropdownMenuTrigger
              class="workdir-action group-more"
              :aria-label="`Actions for ${group.label}`"
              title="More actions"
            >
              <EllipsisIcon class="icon-xs" aria-hidden="true" />
            </DropdownMenuTrigger>
            <!-- Portalled and placed by reka, like every other menu in the app: the rows are
                 absolutely positioned, so the pane under them paints over anything left in place. -->
            <DropdownMenuPortal>
              <DropdownMenuContent
                class="surface-popover marvis-menu group-menu"
                side="bottom"
                align="end"
                :side-offset="4"
                :aria-label="`Actions for ${group.label}`"
              >
                <DropdownMenuItem
                  v-if="group.canAdd"
                  class="menu-item select-none text-left"
                  @select="addWorktree(group)"
                >
                  <GitBranchPlusIcon class="icon-xs" aria-hidden="true" />
                  <span class="menu-item-label">New worktree</span>
                </DropdownMenuItem>
                <!-- Only while there is something to bring back, and the count says how much. -->
                <DropdownMenuItem
                  v-if="group.canRestore"
                  class="menu-item select-none text-left"
                  :aria-label="`Restore ${group.archived} archived worktree${group.archived === 1 ? '' : 's'}`"
                  @select="restoreArchived(group)"
                >
                  <ArchiveRestoreIcon class="icon-xs" aria-hidden="true" />
                  <span class="menu-item-label">Restore archived worktrees</span>
                  <span class="group-menu-count">{{ group.archived }}</span>
                </DropdownMenuItem>
                <!-- Taking the repo off the panel takes its whole list with it. Nothing here deletes
                     a file, and the home directory is the one thing that cannot be closed. -->
                <DropdownMenuItem
                  v-if="group.canClose"
                  class="menu-item select-none text-left group-menu-danger"
                  @select="closeGroup(group)"
                >
                  <FolderMinusIcon class="icon-xs" aria-hidden="true" />
                  <span class="menu-item-label">Remove from panel</span>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPortal>
          </DropdownMenuRoot>
        </div>

        <!-- One group per checkout, and the fold is the group's own: `col` is what hides its
             children and turns its chevron, so the two can never disagree about whether it is
             open. The chevron is the control rather than the whole row because the row itself
             selects the checkout, and folding is not selecting. -->
        <div
          v-for="workdir in group.workdirs"
          :key="workdir.checkout.id"
          class="workdir-checkouts grp"
          :class="{
            col: isCollapsed(workdir),
            'has-active': workdir.items.some((item) => item.active),
            'is-drop-target': dropCheckoutId === workdir.checkout.id,
          }"
          :data-workdir-checkout="workdir.checkout.id"
        >
          <div
            class="workdir-row workdir-parent"
            :class="{ active: workdir.active, selected: workdir.selected, missing: workdir.missing }"
          >
            <button
              type="button"
              class="workdir-fold"
              :aria-expanded="!isCollapsed(workdir)"
              :aria-label="`${isCollapsed(workdir) ? 'Expand' : 'Collapse'} ${workdir.title}`"
              @click="toggleGroup(workdir)"
            >
              <ChevronDownIcon class="chv" aria-hidden="true" />
            </button>

            <button
              type="button"
              class="workdir-select"
              :aria-current="workdir.active ? 'page' : undefined"
              :aria-disabled="workdir.missing || undefined"
              :title="workdirTooltip(workdir.checkout)"
              @click="!workdir.missing && emit('selectCheckout', workdir.checkout.id, hasChanges(workdir))"
            >
              <component :is="workdir.icon" class="workdir-icon" aria-hidden="true" />
              <span class="lbl">
                <span class="nm">{{ workdir.title }}</span>
              </span>
            </button>

            <!-- The row's own trailing slot. The counts live here and nowhere else, in the
                 monospace the reference draws them in and right against the row's edge, and a
                 worktree that can be taken off the panel yields this slot to that action rather
                 than having the action painted over the counts. -->
            <span class="workdir-end" :class="{ 'workdir-end-error': !!workdir.error }">
              <span v-if="workdir.error" class="workdir-end-note">{{ workdir.error }}</span>
              <span v-else-if="workdir.additions || workdir.deletions" class="workdir-diff">
                <span v-if="workdir.additions" class="ad">+{{ workdir.additions }}</span>
                <span v-if="workdir.deletions" class="rm">−{{ workdir.deletions }}</span>
              </span>
            </span>

            <!-- A worktree is the only row with an action of its own, and it is out of flow so it
                 cannot reflow the name beside it: the counts give up the slot's width instead. A
                 repo root's own actions are in the group header above. -->
            <button
              v-if="workdir.worktree && !workdir.home"
              type="button"
              class="workdir-action workdir-close"
              :aria-label="
                workdir.missing
                  ? `Close missing checkout: ${workdir.title}`
                  : `Remove or archive worktree ${workdir.title}`
              "
              :title="workdir.missing ? 'Remove from list' : 'Remove or archive worktree'"
              @click="
                workdir.missing
                  ? emit('closeMissing', workdir.checkout.id)
                  : emit('removeWorktree', workdir.checkout.id)
              "
            >
              <XIcon class="icon-xs" aria-hidden="true" />
            </button>
          </div>

          <!-- The group's terminals, one level in and behind a guide that only appears while the
               pointer is over the group itself, so the line never reads as a selection. The whole
               list is also the rest of the workdir's drop zone: a terminal is dropped on the
               worktree it should belong to, not on whichever of its rows is under it.

               "New terminal" is deliberately not gated on having terminals open: picking a workdir
               no longer opens a terminal by itself, so a workdir with nothing running would
               otherwise offer no way to start one from here. A directory that is gone has no live
               sessions and nothing to run one in. -->
          <div v-if="!workdir.missing" class="kids workdir-kids">
            <template v-for="(item, itemIndex) in workdir.items" :key="item.session.id">
              <!-- The line that says where the terminal lands. It is drawn in the slot itself rather
                 than once at the end of the list, because a line that does not move while the rows
                 above it do says nothing about the position being offered. It is a `workdir-row`
                 because it IS the row at that slot: it takes its box, its indent and its glyph axis
                 from the same rules, and a line with a geometry of its own sat a few pixels left of
                 the terminals above and below it. -->
              <div
                v-if="dropLineBefore(workdir.checkout, itemIndex)"
                class="workdir-row workdir-child terminal-drop-insertion"
                aria-hidden="true"
              >
                <SquareTerminalIcon class="workdir-icon" />
                <span>Drop terminal here</span>
              </div>

              <div
                class="workdir-row workdir-child"
                :data-session-id="item.session.id"
                :class="[
                  `state-${item.state}`,
                  {
                    active: item.active,
                    selected: item.active,
                    'is-being-dragged': pointerDrag?.started && pointerDrag.session.id === item.session.id,
                  },
                ]"
              >
                <!-- Editing swaps the button for the field, rather than nesting an input inside
                   one: a control inside a control cannot be focused or read on its own. The row
                   keeps its shape because both are laid out the same way. -->
                <div v-if="editingId === item.session.id" class="workdir-select">
                  <component :is="item.icon" class="workdir-icon" aria-hidden="true" />
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
                <!-- A terminal moves by mouse after a small pointer threshold; an ordinary click
                   still selects it. The menu remains the keyboard equivalent. -->
                <button
                  v-else
                  type="button"
                  class="workdir-select"
                  :aria-current="item.active ? 'page' : undefined"
                  :aria-label="`Terminal session: ${rowLabel(item)}`"
                  :aria-haspopup="item.destinations.length ? 'menu' : undefined"
                  :aria-expanded="item.destinations.length ? moveMenuFor === item.session.id : undefined"
                  :title="rowLabel(item)"
                  @pointerdown="startPointerDrag(item.session, $event)"
                  @lostpointercapture="onLostPointerCapture"
                  @click="selectSession(item.session.id, $event)"
                  @dblclick="startRename(item.session)"
                  @keydown.f2.prevent="startRename(item.session)"
                  @keydown.shift.f10.prevent="item.destinations.length && openMoveMenu(item.session.id)"
                  @contextmenu.prevent="item.destinations.length && openMoveMenu(item.session.id)"
                >
                  <!-- The glyph carries the state, in colour and in motion: a spinner while a turn
                     runs, the sparkles while it waits for a reply or after a turn failed, a
                     terminal's square while a plain process is up, and the same glyph in grey when
                     nothing is. There is no badge, no chip and no second line, so the icon is the
                     only place the state can live and it has to be right. -->
                  <component :is="item.icon" class="workdir-icon" aria-hidden="true" />
                  <span class="lbl">
                    <span class="nm">{{ item.title }}</span>
                    <span v-if="item.detail" class="dm">{{ item.detail }}</span>
                  </span>
                </button>

                <!-- The time and the cross share this slot, and the slot is exactly as wide as the
                   cross: the time is `display: none` on hover rather than moved, so nothing in the
                   row can shift when the pointer arrives. A row with no session has no time and
                   only ever had the empty slot. -->
                <span class="workdir-end">
                  <span v-if="item.elapsed" class="workdir-end-time">{{ item.elapsed }}</span>
                </span>

                <button
                  type="button"
                  class="workdir-action workdir-close"
                  :aria-label="`Close terminal session: ${item.title}`"
                  :title="`Close ${item.title}`"
                  @click="emit('closeSession', item.session.id)"
                >
                  <XIcon class="icon-xs" aria-hidden="true" />
                </button>

                <!-- The destinations as a menu, for the move a drag cannot make. It hangs below the
                   row rather than pushing it, so a list of worktrees never changes the panel it is
                   read from. -->
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
            </template>

            <div
              v-if="dropLineAfter(workdir.checkout)"
              class="workdir-row workdir-child terminal-drop-insertion"
              aria-hidden="true"
            >
              <SquareTerminalIcon class="workdir-icon" />
              <span>Drop terminal here</span>
            </div>

            <div class="workdir-row workdir-child new-item">
              <button
                type="button"
                class="workdir-select"
                :aria-label="`New terminal for ${workdir.title}`"
                @click="emit('newTerminal', workdir.checkout.id)"
              >
                <!-- The same glyph slot a terminal wears, so this row's label starts on the axis the
                     terminals' labels start on rather than on the branch row's. -->
                <PlusIcon class="workdir-icon" aria-hidden="true" />
                <span class="lbl">
                  <span class="nm">New terminal</span>
                </span>
              </button>
            </div>
          </div>
        </div>
      </div>

      <p v-if="!groups.length" class="pane-state" role="status">
        Your opened folders will appear here. Use Open directory below to add one.
      </p>
    </div>

    <div class="sidebar-footer">
      <div class="sep" />
      <button
        type="button"
        class="workdir-row add-item"
        aria-label="Open directory"
        title="Open directory"
        :disabled="isOpening"
        @click="emit('openFolder')"
      >
        <FolderPlusIcon class="workdir-icon" aria-hidden="true" />
        <span class="lbl">
          <span class="nm">Open directory</span>
        </span>
      </button>
    </div>
    <Teleport to="body">
      <div
        v-if="pointerDrag?.started"
        class="terminal-drag-ghost"
        aria-hidden="true"
        :style="{ left: pointerDrag.x + 14 + 'px', top: pointerDrag.y + 14 + 'px' }"
      >
        <SquareTerminalIcon class="workdir-icon" />
        <span>{{ draggedItem?.title }}</span>
      </div>
    </Teleport>
  </aside>
</template>

<style scoped>
/* ---------------------------------------------------------------------------------------------
   The panel, from the reference: one line per row, dense and flat, the state in the colour of the
   icon. Every rule below is a decision about pixels or about what a row is allowed to claim, and
   the comments say which. Nothing here draws a chip, a badge or a second line.
   --------------------------------------------------------------------------------------------- */

/* Air between two repos, and only between two repos: the gap above each header is what says this is
   a new list. Two checkouts of the same repo have none between them \u2014 they are one list, and a gap
   there would read as a division the repo does not have. */
.workdir-group {
  padding: 2px 0 8px;
}

/* Group header: the repo's name at a size and weight that read as a section, its location
   underneath in the muted 11px the reference uses, and one menu button on the right that appears
   on hover.

   It is set apart by weight and size rather than by a surface: nothing here is a row, so nothing
   here is painted like one. The name is medium rather than bold because a repository is a heading,
   not an announcement. */
.group-heading {
  position: relative;
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 8px 8px 4px 10px;
}

/* The name is the control that folds the repository, so it is a button wearing the heading's own
   type: it takes the row the pointer is over, it says which way it goes with `aria-expanded`, and it
   carries a chevron rather than leaving a reader to infer the state from what happens to be
   underneath it. Reset here rather than inherited, because what it would inherit is the button
   treatment every other control in the app has — a surface and a border a heading must not have. */
.group-heading-text {
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: baseline;
  gap: 8px;
  /* The button's gutter is always there: a header whose text moved as the pointer arrived would
     move the name of everything under it. */
  padding: 0 30px 0 0;
  background: transparent;
  border: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}

/* A repository folded shut is its header and nothing else, the way a folded worktree is its row.
   Nothing marks it: the empty space under the header says it, and a glyph beside the name would put
   an icon on a row that is a heading rather than one of the list's own rows. */
.workdir-group.is-collapsed .workdir-checkouts {
  display: none;
}

.group-name {
  overflow: hidden;
  flex-shrink: 1;
  color: var(--marvis-text);
  font-size: 0.875rem;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-path {
  overflow: hidden;
  flex: 0 1 auto;
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
  text-overflow: ellipsis;
  white-space: nowrap;
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

/* Nothing here positions the menu: reka hangs it below the button and portals it to the body, the
   way every other menu in the app is placed. What is left is what this one says that the others do
   not: a width for whole phrases rather than branch names, the gap its leading icon asks for, the
   icon itself, and the count at the end of a row. The surface comes from `surface-popover` and the
   rows from `.menu-item`, so the two kinds of menu cannot drift apart. */
.group-menu {
  min-width: 224px;
}

.group-menu .menu-item {
  gap: 8px;
}

.group-menu .menu-item .icon-xs {
  flex-shrink: 0;
  color: var(--marvis-text-secondary);
}

.group-menu-count {
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
}

/* The destructive action is set apart by a rule, and is the only place red appears. The colour
   follows the row the way the shared rule does: `data-highlighted` is what reka puts on the row the
   arrows are on, which a CSS `:focus-visible` cannot see, because the menu moves that focus itself. */
.group-menu .group-menu-danger {
  margin-top: 6px;
  border-top: 1px solid var(--marvis-control-hover);
}

.group-menu .group-menu-danger[data-highlighted]:not([data-disabled]),
.group-menu .group-menu-danger:hover {
  color: var(--marvis-danger-fg);
}

/* ---------------------------------------------------------------------------------------------
   The row. One line, 26px, and square like everything else in Marvis: a rounded row carrying a
   two-pixel accent edge down its left is a shape saying "this one" twice, and the edge says it once.
   Every row here — a branch, a terminal, an action — is the same square box, so which of them it is
   is said by the glyph and the name rather than by the outline.
   --------------------------------------------------------------------------------------------- */
.workdir-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  height: 26px;
  padding: 0 8px;
}

/* A group that holds the terminal being read is said by its own row's glyph, never by a surface:
   a second tinted row above the selected one would put two claims about "where you are" in the same
   list. It comes *before* the selection rules below, because on a row that is both the current
   checkout and the selected one the selection has to win: this is context, and context never
   outranks a choice. */
.workdir-checkouts.has-active .workdir-parent .workdir-icon {
  color: var(--marvis-text-secondary);
}

/* Hover and selection are told apart, and this is the whole of the panel's vocabulary for them.
   A hover is the subtle surface and nothing else: no edge and no accent, because the row under the
   pointer is a row being acted on, not a row being chosen. Selection is a different surface again,
   plus a two-pixel accent edge down its own left, so it can be picked out from across the panel
   without reading the ink. The edge is an inset shadow, so it is drawn inside the row's box and
   cannot spill onto the group guide beside it. */
.workdir-row:hover {
  background: var(--marvis-el-hover);
}

/* Selection is one row in the whole panel, and it is whichever row carries `selected`: the terminal
   being read, or the branch when no terminal of it is selected. A checkout that merely HOLDS the
   selected terminal is context, and it used to wear the same tint and the same blue edge, so the
   panel showed two rows claiming the selection at once. */
.workdir-row.selected {
  background: var(--marvis-el-selected);
  color: var(--marvis-text);
  box-shadow: inset 2px 0 0 var(--marvis-accent);
}

/* There is deliberately NO rule here for the selected row's glyph. A rule would outrank every
   `.state-*` rule below — `.workdir-row.selected .workdir-icon` is three classes against their two —
   so selecting a terminal with a process in it repainted its green glyph in primary ink, and the row
   said, for as long as it was selected, that nothing was running in it. Selection is not a state: it is
   already said by the tint and the accent edge, and the glyph keeps saying what it means. The grey of
   an idle row is lifted below instead, so it reads on the selected tint without repainting anything. */

/* The repo header is a row too, and hovers like one: the same surface, the same padding, the same
   square corners. The surface belongs to the heading itself rather than to the button inside it, so
   the empty stretch past the name is part of the click that folds the repository. It has no selected
   state, because what is selected is a checkout and that is the row underneath it. */
.group-heading:hover {
  background: var(--marvis-el-hover);
}

/* The panel's own type: the reference's 13px, which is a step under the interface's 14px because
   every row in here is one line and the rows are read in columns rather than one at a time. */
.app-sidebar {
  font-size: 13px;
}

.sidebar-scroll {
  min-height: 0;
  flex: 1;
  overflow-y: auto;
  /* The right padding is the scrollbar's: macOS draws its own overlay scrollbar over the content,
     so a row whose name and counts end at the edge are read through it. */
  padding-right: 8px;
}

/* The fold, which is a button of its own rather than the whole row: the row selects the checkout,
   and folding a group is not selecting it. */
.workdir-fold {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-left: -4px;
  flex-shrink: 0;
  padding: 0;
  background: transparent;
  border: none;
  color: var(--marvis-text-faint);
  cursor: pointer;
}

.workdir-fold:hover {
  color: var(--marvis-text);
}

/* One turn, and the only motion in the panel. */
.chv {
  transition: transform 0.12s ease;
}

.col .chv {
  transform: rotate(-90deg);
}

/* A folded group is its row and nothing else. The chevron is what says it, and it says it by
   pointing at the list that is not there. */
.col > .kids {
  display: none;
}

/* ---------------------------------------------------------------------------------------------
   The group's children, and the guide that says they belong to it.
   --------------------------------------------------------------------------------------------- */
.kids {
  display: flex;
  flex-direction: column;
  margin-left: 14px;
  /* Transparent at rest and taken only while the pointer is over the group. A guide that is always
     there is a second vertical line in the panel, and a reader cannot tell an indent from a
     selection; this one is only ever drawn while it is being pointed at, which is also the only
     moment its meaning is being used. */
  border-left: 1px solid transparent;
  transition: border-color 0.15s ease;
}

.grp:hover > .kids {
  border-left-color: var(--marvis-border);
}

.kids .workdir-row {
  padding-left: 12px;
}

/* ---------------------------------------------------------------------------------------------
   The label: a name that gives way last and a detail that gives way first.
   --------------------------------------------------------------------------------------------- */
.workdir-select {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 7px;
  height: 100%;
  padding: 0;
  background: transparent;
  border: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}

.lbl {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 6px;
  overflow: hidden;
  white-space: nowrap;
}

/* The name gives way, but last: `flex: 0 1 auto` lets it shrink, and `min-width: 0` is what lets it
   be narrower than its text so the ellipsis has a width to work in at all. */
.nm {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  color: var(--marvis-text);
  text-overflow: ellipsis;
}

/* The detail goes first, and this is the whole rule for it.
   `flex: 1 1 0` looked like it did that and did not: a zero basis means the detail's own share of the
   shrink is zero, so a long name absorbed every pixel taken and the detail collapsed to nothing rather
   than being cut — `Review the duplicated rows in the sidebar` with no mode beside it at all, which is
   the one thing a two-part label cannot do. So the detail is sized by its own contents, like the name.

   Shrink 2 against the name's 1 is what makes it go first: both lose room in proportion to how much
   each brought, and the detail gives up twice as fast, so its tail is cut before the name is touched.
   There is deliberately NO `max-width` here. A cap was tried — `50%` of `.lbl` — and it cut a branch
   name to `feat/feedbac…` on a 203px label that had 176px spare, purely because a cap is a ceiling
   rather than a share: it trims a row with nothing competing for the space. Without one, the detail
   shows in full whenever the row can hold it, and is cut only when something is actually in the way.

   `min-width` is the other half and it is a floor, not a ceiling: the detail is never trimmed to
   nothing (measured at 2px on a 240px panel, which is not a truncation, it is a deletion), and once
   it reaches the floor the NAME is what gives way — `min-width: 0` on `.nm` is what lets it be
   narrower than its text so the ellipsis has a width to work in. Six characters is enough for a mode
   like `plan` or `coder` to read whole, and a longer one still gets a cut. */
.dm {
  flex: 0 2 auto;
  min-width: 6ch;
  overflow: hidden;
  color: var(--marvis-text-faint);
  font-size: 0.75rem;
  text-overflow: ellipsis;
}

/* The glyph, and the whole of the row's state. 14px is one line of this type, so it sits on the
   name's own line box with no nudge. */
.workdir-icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: var(--marvis-text-faint);
}

/* A row whose directory is gone stays listed to say so and to be closed. Its label reads as
   unavailable: nothing behind it can be selected, and the single action beside it is the only
   thing the row still does. */
.workdir-parent.missing .workdir-select {
  color: var(--marvis-text-disabled);
  cursor: not-allowed;
}

.workdir-parent.missing .workdir-icon,
.workdir-parent.missing .nm {
  color: var(--marvis-text-disabled);
}

.workdir-select[aria-disabled="true"] {
  color: var(--marvis-text-disabled);
  cursor: not-allowed;
}

/* ---------------------------------------------------------------------------------------------
   The trailing slot: a row's time, or a branch's counts, and the cross that replaces it on hover.
   --------------------------------------------------------------------------------------------- */
.workdir-end {
  position: relative;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 20px;
  color: var(--marvis-text-faint);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
}

.workdir-end-note {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workdir-end-error {
  color: var(--marvis-danger-fg);
}

/* The counts: monospaced, because they are read down a column of rows rather than one at a time,
   and a proportional digit moves them sideways. Green for additions and red for deletions, which
   are the two colours the reference uses and the two the palette already has. */
.workdir-diff {
  display: flex;
  gap: 5px;
  font-family: var(--marvis-font);
}

.ad {
  color: var(--marvis-content-added);
}

.rm {
  color: var(--marvis-content-removed);
}

/* The cross, out of flow at the trailing slot's own width so the row never reflows and the name
   never moves. It paints the surface of the row it belongs to — the panel at rest, the hover surface
   under the pointer, the selected tint on a selected row — so whatever it does land over, it lands
   over the row rather than letting text show through it.

   At rest it is `opacity: 0`: invisible, and nothing around it either, because the reference draws
   no cross on a resting row and a grey cell at the end of every branch is an object that list does
   not have. It is faded rather than `display: none`, which is the one deliberate departure from the
   reference and the reason is the tab order: a removed button cannot be reached, so a close a
   keyboard cannot find is a close that does not exist for half the people who use this. Invisible and
   focusable is the pair that works, and the rule below is what makes it visible the moment it is
   either pointed at or focused. */
.workdir-close {
  position: absolute;
  top: 50%;
  /* The row's right edge, so it lands exactly on top of whatever sits there — the diff figures or the
     elapsed time — the way an actions menu opens over a row rather than pushing it along. */
  right: 8px;
  width: 20px;
  height: 20px;
  transform: translateY(-50%);
  /* Above the trailing slot, which is itself positioned: two positioned siblings would be painted in
     DOM order anyway, and the cross is last, but stacking is stated rather than inferred so moving a
     child in the template cannot silently put the figures on top of the glyph. */
  z-index: 1;
  background: var(--marvis-bg-1);
  opacity: 0;
  transition: opacity 0.12s ease;
}

/* The surface the cross paints is the row's own, and it follows the row: the panel at rest, the hover
   surface under the pointer, the selected tint on a selected row. That opaque fill is what makes it
   read as a menu opening *on* the row rather than as content moving underneath it — nothing shows
   through it, and the glyph is never left under the figures it is covering. The two rules are one
   selector apart and the selected one comes second, so a hovered selected row wears its own tint. */
.workdir-row:hover > .workdir-close {
  background: var(--marvis-el-hover);
}

.workdir-row.selected > .workdir-close {
  background: var(--marvis-el-selected);
}

/* Visible whenever it is pointed at or focused — three ways, and no fourth. */
.workdir-row:hover > .workdir-close,
.workdir-row:focus-within > .workdir-close,
.workdir-close:focus-visible {
  opacity: 1;
}

/* **Nothing on a row moves when the pointer arrives.** There is no yield rule, and that is the whole
   of it: the trailing slot's box is identical at rest and hovered, so a branch's change figures and a
   terminal's elapsed time stay exactly where they were drawn. The cross covers what is under it
   rather than asking it to step aside, because a count that slides left the instant the pointer lands
   is a number a reader has to re-find at the exact moment they are looking at the row. */

/* ---------------------------------------------------------------------------------------------
   The state, in the icon's colour. Five answers, and the words are in the row's accessible name.
   --------------------------------------------------------------------------------------------- */
.state-working .workdir-icon {
  color: var(--marvis-accent);
  animation: row-spin 1.1s linear infinite;
}

.state-waiting .workdir-icon {
  color: var(--marvis-warning);
}

.state-failed .workdir-icon {
  color: var(--marvis-danger-fg);
}

.state-running .workdir-icon {
  color: var(--marvis-success);
}

/* Idle is the one state with no colour of its own, so its ink is the panel's own muted foreground —
   lifted one step from the faintest token, because this glyph now has to read on the selected row's
   tint as well as on the panel, and faint does not. Nothing else about the state is touched. */
.state-idle .workdir-icon {
  color: var(--marvis-text-muted);
}

@keyframes row-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .state-working .workdir-icon {
    animation: none;
  }

  .chv {
    transition: none;
  }
}

/* ---------------------------------------------------------------------------------------------
   The two rows that add something rather than name what exists.
   --------------------------------------------------------------------------------------------- */
.add-item,
.new-item {
  color: var(--marvis-text-faint);
}

.add-item .workdir-icon,
.new-item .workdir-icon,
.add-item .nm,
.new-item .nm {
  color: inherit;
}

.add-item:hover,
.new-item:hover,
.add-item:focus-visible,
.new-item:focus-visible {
  color: var(--marvis-text);
}

/* The hairline the reference draws between the list and the panel's own last action. */
.sep {
  height: 1px;
  margin: 6px 8px;
  background: var(--marvis-border);
}

.sidebar-footer {
  flex-shrink: 0;
}

/* ---------------------------------------------------------------------------------------------
   The rename field takes the row's own type so the text does not jump when it appears.
   --------------------------------------------------------------------------------------------- */
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

/* ---------------------------------------------------------------------------------------------
   Row actions, and the button every one of them wears. The focus case is not a courtesy: a
   keyboard reaches these with Tab, and a button that only appears on hover is a button a keyboard
   user cannot see.
   --------------------------------------------------------------------------------------------- */
.workdir-action {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  background: transparent;
  border: none;
  color: var(--marvis-text-secondary);
  cursor: pointer;
}

.workdir-action:hover {
  background: var(--marvis-control-hover);
  color: var(--marvis-text);
}

/* The drop target, and the row under a drag that is being held over the panel. */
.workdir-checkouts.is-drop-target > .workdir-parent {
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

/* The destination list, hanging below the row it belongs to. It is out of flow so a list of
   worktrees does not push the rows after it, and it does not fade with a hover: a menu that
   disappears when the pointer leaves the row cannot be read. */
.move-menu {
  position: absolute;
  top: 26px;
  left: 12px;
  z-index: 20;
  min-width: 200px;
}

/* The drop line is a row of the list, and says nothing but what it is. Its box, its indent and its
   glyph axis come from `.workdir-row` and `.kids .workdir-row` — which is the whole point: it used
   to be a box with a margin of its own, four pixels left of the terminals above and below it, so the
   glyph it drew on the cursor's way in was not on the axis of the row it was about to become. */
.terminal-drop-insertion {
  /* An outline rather than a border, for one measurable reason: a border takes a pixel of the row's
     own content box, so the line's glyph would sit one pixel right of every terminal's — the exact
     misalignment the row classes were brought in to remove. Drawn inside the box, it costs nothing. */
  outline: 1px dashed var(--marvis-accent);
  outline-offset: -1px;
  color: var(--marvis-accent);
  font-size: 0.6875rem;
}

.terminal-drag-ghost {
  position: fixed;
  z-index: 1000;
  display: flex;
  align-items: center;
  /* The same row again, this time under the pointer rather than in the list, so it carries the same
   * indent the row it came from wears. The pointer sits inside it at an offset, as it does in the
   * reference; the glyph does not move away from the column it will land in.
   *
   * The type is named here rather than inherited, because this box is teleported to the body and
   * therefore sits outside `.app-sidebar` — which is where the panel's 13px lives. Inheriting the
   * body's size made the dragged row's name larger than the row it was a copy of, which is the same
   * misalignment as an icon off its column and just as wrong. */
  gap: 7px;
  height: 26px;
  padding: 0 12px;
  font-family: inherit;
  font-size: 13px;
  /* Never wider than the panel's own minimum, so the ghost stays a chip over the workspace rather
     than a banner drawn across it — and so it is never wider than the row it is a copy of. */
  max-width: 240px;
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
</style>
