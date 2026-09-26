<script setup lang="ts">
import { computed, toRef } from "vue";
import {
  Folder as FolderIcon,
  FolderGit2 as FolderGit2Icon,
  GitBranch as GitBranchIcon,
  GitFork as GitForkIcon,
  Plus as PlusIcon,
  SquareTerminal as SquareTerminalIcon,
  Trash as TrashIcon,
  X as XIcon,
} from "@lucide/vue";
import type { Checkout, Repo, Session, TerminalSessionStatus } from "../domain/workspace";
import { useDiffStats } from "../presentation/diff-stats";

defineOptions({ name: "FolderSidebar" });

// `activityByCheckout`, `locateMissing` and `closeMissing` are still bound by App.vue: the sidebar
// stopped representing them (C.1, C.2), so the wiring is dropped in the phase that owns App.vue.
const props = withDefaults(
  defineProps<{
    repos: Repo[];
    activeCheckoutId: string | null;
    activeSessionId: string | null;
    isOpening: boolean;
    activityByCheckout?: Record<string, string[]>;
    sessionRuntimeStatuses?: Record<string, TerminalSessionStatus>;
  }>(),
  {
    activityByCheckout: () => ({}),
    sessionRuntimeStatuses: () => ({}),
  },
);

const emit = defineEmits<{
  openFolder: [];
  selectCheckout: [checkoutId: string];
  selectSession: [sessionId: string];
  locateMissing: [checkoutId: string];
  closeMissing: [checkoutId: string];
  createWorktree: [checkoutId: string];
  newTerminal: [checkoutId: string];
  removeWorktree: [checkoutId: string];
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
  branch?: string;
  /** The checkout's own line counts, absent when Git has none to show. */
  additions?: number;
  deletions?: number;
  kind: IconKind;
  /** Repo roots can host a new worktree. */
  gitdir: boolean;
  /** Worktrees can be removed; repo roots cannot. */
  worktree: boolean;
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

function toWorkdir(repo: Repo, checkout: Checkout): Workdir {
  const isGit = repo.kind === "git";
  const counts = isGit ? diffStats.checkoutTotals[checkout.id] : undefined;
  return {
    checkout,
    // The repo root is "Base" and carries its branch aside; a worktree is named by its branch.
    title: checkout.isPrimary ? "Base" : checkout.branch || checkout.path,
    branch: checkout.isPrimary ? checkout.branch : undefined,
    additions: counts?.additions || undefined,
    deletions: counts?.deletions || undefined,
    kind: !isGit ? "folder" : checkout.isPrimary ? "git" : "worktree",
    gitdir: isGit && checkout.isPrimary,
    worktree: isGit && !checkout.isPrimary,
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
    <div class="min-h-0 flex-1 overflow-y-auto">
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
                :title="workdir.checkout.path"
                @click="emit('selectCheckout', workdir.checkout.id)"
              >
                <component :is="icons[workdir.kind]" class="workdir-status-icon" aria-hidden="true" />
                <div class="workdir-main">
                  <div class="workdir-title">
                    <span class="workdir-name">{{ workdir.title }}</span>
                    <div v-if="workdir.branch" class="workdir-branch">
                      <span class="meta-dot">·</span>
                      <GitBranchIcon class="icon-xxs" aria-hidden="true" />
                      <span>{{ workdir.branch }}</span>
                    </div>
                  </div>
                  <div class="workdir-meta">
                    <span v-if="workdir.additions" class="diff-add">+{{ workdir.additions }}</span>
                    <span v-if="workdir.deletions" class="diff-del">-{{ workdir.deletions }}</span>
                  </div>
                </div>
              </button>

              <div class="workdir-actions">
                <button
                  v-if="workdir.worktree"
                  type="button"
                  class="workdir-action workdir-action-danger"
                  :aria-label="`Remove worktree ${workdir.title}`"
                  title="Remove worktree"
                  @click="emit('removeWorktree', workdir.checkout.id)"
                >
                  <TrashIcon class="icon-xs" aria-hidden="true" />
                </button>
                <button
                  v-if="workdir.gitdir"
                  type="button"
                  class="workdir-action"
                  :aria-label="`Add worktree from ${workdir.branch || workdir.checkout.path}`"
                  title="Add worktree"
                  @click="emit('createWorktree', workdir.checkout.id)"
                >
                  <PlusIcon class="icon-xs" aria-hidden="true" />
                </button>
              </div>
            </div>
          </div>

          <!-- Child items: the same row as the workdir, minus the diff, plus a close -->
          <div class="workdir-items">
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
                  <component
                    :is="icons.terminal"
                    class="workdir-status-icon"
                    :class="{ 'is-running': !item.active && !item.exited }"
                    aria-hidden="true"
                  />
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
  gap: 8px;
  padding: 0;
  background: transparent;
  border: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}

.workdir-select.new-item .workdir-name {
  color: var(--marvis-text-dim);
  font-size: 12px;
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

/* A live session is one step brighter than an exited one */
.workdir-status-icon.is-running {
  color: var(--marvis-text-secondary);
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

/* The flex box only clips, so the ellipsis lives on the name itself */
.workdir-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.workdir-branch {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--marvis-text-faint);
  font-size: 11px;
  flex-shrink: 0;
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

.workdir-item:hover .workdir-meta {
  display: none;
}

.meta-dot {
  opacity: 0.6;
}
</style>
