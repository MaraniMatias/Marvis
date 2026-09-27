<script setup lang="ts">
import { computed, toRef } from "vue";
import {
  Folder as FolderIcon,
  FolderGit2 as FolderGit2Icon,
  GitFork as GitForkIcon,
  Plus as PlusIcon,
  SquareTerminal as SquareTerminalIcon,
  Trash as TrashIcon,
  X as XIcon,
} from "@lucide/vue";
import type { Checkout, Repo, Session, TerminalSessionStatus } from "../domain/workspace";
import { workdirTitle } from "../domain/workspace";
import { useDiffStats } from "../presentation/diff-stats";

defineOptions({ name: "FolderSidebar" });

const props = withDefaults(
  defineProps<{
    repos: Repo[];
    activeCheckoutId: string | null;
    activeSessionId: string | null;
    isOpening: boolean;
    sessionRuntimeStatuses?: Record<string, TerminalSessionStatus>;
  }>(),
  {
    sessionRuntimeStatuses: () => ({}),
  },
);

const emit = defineEmits<{
  openFolder: [];
  selectCheckout: [checkoutId: string, hasChanges: boolean];
  selectSession: [sessionId: string];
  createWorktree: [checkoutId: string];
  newTerminal: [checkoutId: string];
  removeWorktree: [checkoutId: string];
  closeMissing: [checkoutId: string];
  closeSession: [sessionId: string];
}>();

/** The icon roles a workdir row can ask for, resolved to a real lucide component. */
const icons = {
  git: FolderGit2Icon,
  worktree: GitForkIcon,
  folder: FolderIcon,
  terminal: SquareTerminalIcon,
};

type IconKind = keyof typeof icons;

interface WorkdirItem {
  session: Session;
  active: boolean;
  exited: boolean;
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
  /** Repo roots can host a new worktree. */
  gitdir: boolean;
  /** Worktrees can be removed; repo roots cannot. */
  worktree: boolean;
  /**
   * The directory is gone, so the row keeps only its reason for existing and its one way out
   * (closing it). Nothing behind it can be selected, run or created.
   */
  missing: boolean;
  active: boolean;
  items: WorkdirItem[];
}

// The sidebar names every checkout at once, so it asks for the totals of all of them in one
// call rather than per row. The active checkout is named too, so the Changes tab of the
// inspector shares the same refresh instead of running a second listener.
const diffStats = useDiffStats(toRef(props, "repos"), toRef(props, "activeCheckoutId"));

const groups = computed(() =>
  props.repos.map((repo) => ({
    label: repo.name,
    workdirs: repo.checkouts.map((checkout) => toWorkdir(repo, checkout)),
  })),
);

/**
 * Whether the row is advertising changes, which is what makes clicking it open the full diff.
 * The counts are the same ones the row paints, so the click and the numbers cannot disagree.
 */
function hasChanges(workdir: Workdir): boolean {
  return Boolean(workdir.additions || workdir.deletions);
}

/**
 * A checkout's name in two halves, so that the half which names the branch survives the row's
 * width. The tail is what tells two branches of the same repo apart; the head is the ticket
 * prefix every one of them carries, and the first thing a cut name would otherwise throw away.
 *
 * The head ends at the first `-` that follows a run of digits in the segment after the last `/` —
 * the boundary a ticket-based branch name puts there. A name with no ticket in it, like `main`
 * or `release/1.2.0`, has no head to keep and is drawn whole, which is all the room it needs.
 */
function branchLabel(title: string): { head: string; tail: string } {
  const segment = title.slice(title.lastIndexOf("/") + 1);
  const ticket = /^(.*?\d)-(.*)$/.exec(segment);
  if (!ticket) return { head: "", tail: title };
  return { head: title.slice(0, title.length - segment.length) + ticket[1] + "-", tail: ticket[2] };
}

/** The row's name at full length, which the row cannot fit, and where the checkout lives. */
function workdirTooltip(checkout: Checkout): string {
  return checkout.branch ? `${checkout.branch} — ${checkout.path}` : checkout.path;
}

function toWorkdir(repo: Repo, checkout: Checkout): Workdir {
  const isGit = repo.kind === "git";
  const counts = isGit ? diffStats.checkoutTotals[checkout.id] : undefined;
  const title = workdirTitle(checkout);
  return {
    checkout,
    // A git repo root is named by the branch it is on, the same as a worktree. The only
    // checkouts with no branch to show — a plain folder, or a repo on a detached HEAD — keep
    // the plain "Base".
    title,
    label: branchLabel(title),
    additions: counts?.additions || undefined,
    deletions: counts?.deletions || undefined,
    // The one failure that belongs to a single workdir (E.4): it names the checkout whose
    // directory is gone, so it belongs in that row and not in a toast about the window.
    error: checkout.isMissing ? "Directory missing" : undefined,
    kind: !isGit ? "folder" : checkout.isPrimary ? "git" : "worktree",
    gitdir: isGit && checkout.isPrimary,
    worktree: isGit && !checkout.isPrimary,
    missing: checkout.isMissing,
    active: checkout.id === props.activeCheckoutId,
    items: checkout.sessions.map((session) => ({
      session,
      active: session.id === props.activeSessionId,
      exited: sessionState(session) === "exited",
    })),
  };
}

function sessionState(session: Session) {
  return props.sessionRuntimeStatuses[session.id]?.state ?? (session.status === "active" ? "running" : "exited");
}
</script>

<template>
  <aside class="app-sidebar flex h-full min-h-0 flex-col border-r text-sm">
    <!-- The right padding is the scrollbar's: macOS draws its own overlay scrollbar over the
         content, so a row whose title and counts end at the edge are read through it. -->
    <div class="min-h-0 flex-1 overflow-y-auto pr-2">
      <div v-for="group in groups" :key="group.label" class="workdir-group">
        <div class="group-header">{{ group.label }}</div>

        <template v-for="workdir in group.workdirs" :key="workdir.checkout.id">
          <div
            class="workdir-item"
            :class="{ active: workdir.active, 'has-active': workdir.items.some((item) => item.active) }"
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

              <div class="workdir-actions">
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
                <button
                  v-if="workdir.worktree && !workdir.missing"
                  type="button"
                  class="workdir-action workdir-action-danger"
                  :aria-label="`Remove worktree ${workdir.title}`"
                  title="Remove worktree"
                  @click="emit('removeWorktree', workdir.checkout.id)"
                >
                  <TrashIcon class="icon-xs" aria-hidden="true" />
                </button>
                <button
                  v-if="workdir.gitdir && !workdir.missing"
                  type="button"
                  class="workdir-action"
                  :aria-label="`Add worktree from ${workdir.checkout.branch || workdir.checkout.path}`"
                  title="Add worktree"
                  @click="emit('createWorktree', workdir.checkout.id)"
                >
                  <PlusIcon class="icon-xs" aria-hidden="true" />
                </button>
              </div>
            </div>
          </div>

          <!-- Child items: the same row as the workdir, minus the diff, plus a close.
               "New terminal" is deliberately NOT gated on having terminals open, unlike the
               mockup: picking a workdir no longer opens a terminal by itself, so a workdir
               with nothing running would otherwise offer no way to start one from here. A
               directory that is gone has no live sessions and nothing to run one in. -->
          <div v-if="!workdir.missing" class="workdir-items">
            <div
              v-for="item in workdir.items"
              :key="item.session.id"
              class="workdir-item workdir-child"
              :class="{ active: item.active }"
            >
              <div class="workdir-row">
                <button
                  type="button"
                  class="workdir-select"
                  :aria-current="item.active ? 'page' : undefined"
                  :aria-label="`Terminal session: ${item.session.name}`"
                  :title="item.session.name"
                  @click="emit('selectSession', item.session.id)"
                >
                  <component :is="icons.terminal" class="workdir-status-icon" aria-hidden="true" />
                  <div class="workdir-main">
                    <div class="workdir-title">
                      <span class="workdir-name">{{ item.session.name }}</span>
                    </div>
                  </div>
                </button>

                <div class="workdir-actions">
                  <button
                    type="button"
                    class="workdir-action"
                    :aria-label="`Close terminal session: ${item.session.name}`"
                    :title="`Close ${item.session.name}`"
                    @click="emit('closeSession', item.session.id)"
                  >
                    <XIcon class="icon-xs" aria-hidden="true" />
                  </button>
                </div>
              </div>
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
  </aside>
</template>

<style scoped>
.workdir-group {
  padding: 2px 0 6px;
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

/* Child items: one level in from the workdir row */
.workdir-child {
  padding-left: 24px;
}

/* One action instead of two, hence a narrower hover gutter */
.workdir-child:hover .workdir-select {
  padding-right: 26px;
}

/* The current item lifts off the hover surface and takes the accent icon */
.workdir-child.active {
  background: var(--marvis-border);
}

.workdir-child.active .workdir-status-icon {
  color: var(--marvis-accent);
}

.workdir-item:hover {
  background: var(--marvis-bg-2);
}

/* An active child marks its workdir the same way, minus the accent */
.workdir-item.has-active {
  background: var(--marvis-border);
  box-shadow: inset 2px 0 0 var(--marvis-text-secondary);
}

.workdir-item.has-active .workdir-status-icon {
  color: var(--marvis-text-secondary);
}

.workdir-item.active {
  background: var(--marvis-bg-2);
  box-shadow: inset 2px 0 0 var(--marvis-accent);
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
  opacity: 0.5;
  cursor: default;
}

.workdir-select.new-item .workdir-name {
  color: var(--marvis-text-dim);
  font-size: 12px;
}

/* A ".new-item" label is 12px, so its icon drops to match. At 14px the glyph outweighed its
   own text and the pair read as detached. */
.workdir-select.new-item .workdir-status-icon {
  width: 12px;
  height: 12px;
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

/* Make room under the overlay so a long title ellipsizes instead of running
   beneath the icons. Two icons plus the gap is the widest strip a row can have. */
.workdir-item:hover .workdir-select {
  padding-right: 46px;
  transition: padding-right 0.12s ease;
}

.workdir-action {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  background: transparent;
  border: none;
  border-radius: var(--marvis-radius);
  color: var(--marvis-text-secondary);
  cursor: pointer;
}

.workdir-action:hover {
  background: var(--marvis-border);
  color: var(--marvis-text);
}

.workdir-action-danger:hover {
  color: var(--marvis-red);
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

/* The head never gives way: it is the same width on every branch of a repo, so shrinking it only
   throws away room the tail could have used. The cap is in characters rather than in percent of
   the row, because it is a budget for a prefix: it clears the ticket prefixes in use, and a
   prefix long enough to miss it is cut with an ellipsis instead of taking the row. */
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
  font-size: 11px;
}

/* Hover drops the counts to clear the row actions. An error stays: the hover gutter already
   reserves the room, and a row that is being hovered at is exactly the row being read. */
.workdir-item:hover .workdir-meta:not(.workdir-meta-error) {
  display: none;
}

.workdir-meta-error {
  color: var(--marvis-red);
}
</style>
