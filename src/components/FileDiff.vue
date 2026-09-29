<script setup lang="ts">
/* eslint-disable vue/html-indent, vue/html-closing-bracket-newline, vue/html-self-closing */
import { Check as CheckIcon, ChevronDown as ChevronDownIcon } from "@lucide/vue";
import { DiffFile, DiffModeEnum, DiffViewWithMultiSelect } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, inject, nextTick, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff, GitDiffPageLine } from "../domain/git";
import { ALL_CHANGES_LABEL } from "../domain/main-document";
import { isIpcError } from "../domain/ipc";
import { agentAttention, sortAgentSessions } from "../domain/agent";
import { buildDiffLineTexts, isReviewableNote, reviewRangeCode } from "../domain/review";
import type { AnchorOutcome, ReviewNote, ReviewSide } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";
import { REVIEW_SENDER } from "../presentation/review-notes";
import type { ReviewAnchorCheck } from "../lib/ipc";
import { getGitDiff } from "../lib/ipc";
import { DIFF_ROW_HEIGHT, useLargeDiff } from "./use-large-diff";
import ReviewComposer from "./ReviewComposer.vue";
import ReviewNoteList from "./ReviewNoteList.vue";

const props = withDefaults(
  defineProps<{
    checkout: Checkout;
    gitSnapshot: ActiveGitSnapshot;
    review: Pick<
      ActiveReviewNotes,
      "notes" | "addNote" | "updateNote" | "deleteNote" | "verifyAnchors" | "clearOutdated" | "resolveNote"
    >;
    /** Null is the whole change set: one diff per changed file, stacked in status order. */
    path: string | null;
    scrollTop: number;
    /** Nested in the change-set stack, where the row above already names the file. */
    embedded?: boolean;
  }>(),
  { embedded: false },
);
const emit = defineEmits<{
  ready: [path: string];
  scrollPositionChanged: [top: number];
}>();

const diff = shallowRef<GitFileDiff | null>(null);
const diffHunks = shallowRef<Array<{ title: string; file: DiffFile }>>([]);
const collapsedHunks = ref<number[]>([]);
const diffState = ref<"idle" | "loading" | "ready" | "error">("idle");
/** Why the diff is not on screen. It is the panel's whole content, so it is drawn here. */
const diffError = ref("");
const hasTextHunks = computed(() => Boolean(diff.value?.patch.includes("@@") || diff.value?.totalLines));
const showNoTextHunks = computed(
  () =>
    diffState.value === "ready" &&
    !diff.value?.isBinary &&
    diff.value?.symlinkTarget === undefined &&
    !diff.value?.tooLarge &&
    !hasTextHunks.value,
);
const diffViewport = ref<HTMLElement | null>(null);
const diffScrollTop = ref(props.scrollTop);
const selectedPath = ref<string | null>(null);
/** The files of the whole change set the user has opened. */
const expandedPaths = ref<string[]>([]);
const changedFiles = computed(() => props.gitSnapshot.status?.files ?? []);
const title = computed(() => props.path ?? ALL_CHANGES_LABEL);
const branch = computed(() => props.gitSnapshot.status?.branch ?? props.gitSnapshot.status?.head ?? "");
const largeDiff = useLargeDiff(() => props.checkout.id, selectedPath, diff, collapsedHunks, diffScrollTop);
const { diffPages, largeDiffLineCount, loadVisiblePages, visibleLargeDiffWindow } = largeDiff;
const draft = ref<{ side: ReviewSide; lineStart: number; lineEnd: number } | null>(null);
const draftError = ref("");
/** Diff texts keyed by `${side}:${line}`, for the whole file or all retained virtual pages. */
const lineTexts = computed(() => {
  if (!diff.value?.large || diff.value.tooLarge) return buildDiffLineTexts(diff.value?.patch ?? "");
  const texts = new Map<string, string>();
  for (const page of Object.values(diffPages.value)) {
    for (const line of page.lines) {
      const anchor = rowAnchor(line);
      if (anchor) texts.set(`${anchor.side}:${anchor.line}`, line.text.slice(1));
    }
  }
  return texts;
});
const fileNotes = computed(() => props.review.notes.filter((note) => note.path === props.path));
/** Notes per line, shaped for the diff view's `extendData` slot. */
const extendData = computed(() => {
  const data: { oldFile: Record<string, { data: ReviewNote[] }>; newFile: Record<string, { data: ReviewNote[] }> } = {
    oldFile: {},
    newFile: {},
  };
  for (const note of fileNotes.value) {
    const target = note.side === "old" ? data.oldFile : data.newFile;
    const key = String(note.lineStart);
    target[key] = { data: [...(target[key]?.data ?? []), note] };
  }
  return data;
});
/** Anchor text of every note whose line the diff currently renders, or null while it does not. */
function anchorText(note: ReviewNote): string | null {
  const key = `${note.side}:${note.lineStart}`;
  return lineTexts.value.has(key) ? (lineTexts.value.get(key) ?? "") : null;
}
const anchorChecks = computed(() => {
  const checks: ReviewAnchorCheck[] = [];
  for (const note of fileNotes.value) {
    const currentCode = anchorText(note);
    if (currentCode !== null) checks.push({ id: note.id, currentCode });
  }
  return checks;
});
/** A large diff only knows the lines it has loaded, so it cannot call a line deleted. */
const wholeFileInMemory = computed(() => !diff.value?.large || Boolean(diff.value?.tooLarge));
/**
 * What the diff can prove about each note's anchor, for the notes the agent has seen.
 *
 * `outdated` is the backend's hash comparison and is authoritative; whether the line is
 * still present is what only this view can answer, because only it has the diff.
 */
const anchorOutcomes = computed(() => {
  const outcomes = new Map<string, AnchorOutcome>();
  for (const note of fileNotes.value) {
    const currentCode = anchorText(note);
    if (currentCode === null) {
      outcomes.set(note.id, wholeFileInMemory.value ? "missing" : "unknown");
      continue;
    }
    outcomes.set(note.id, note.outdated ? "changed" : "unchanged");
  }
  return outcomes;
});
/**
 * Notes the agent has seen whose line is gone from the diff.
 *
 * A deleted line has no row to hang a note on, so without this the one case where "the
 * agent acted on it" is most likely would be the one case the user could not see. Only
 * offered when the whole diff is in memory, because a large diff that has not loaded a line
 * cannot claim the line is gone.
 */
const notesWithoutLine = computed(() =>
  fileNotes.value.filter((note) => note.status === "sent" && wholeFileInMemory.value && anchorText(note) === null),
);

// The diff owns which notes go out, while the shell owns what the selected destination does with
// them. The distinction matters here because only an agent handoff can be queued or interrupted.

/** Absent when no shell is above this diff, which is the case in the diff's own tests. */
const sender = inject(REVIEW_SENDER, null);
const includeOutdated = ref(false);
/** Open while the target is working and the user has not yet chosen what to do about it. */
const busyChoiceOpen = ref(false);
const targetOpen = ref(false);
const targetQuery = ref("");
const targetIndex = ref(0);
/** A list this short is picked by looking at it; past this it earns a search of its own. */
const searchesTargets = computed(() => sender?.target === "opencode" && (sender.sessions.length ?? 0) > 5);
/** Newest first, the order the rest of the app reads sessions in. */
const targetRows = computed(() => {
  const rows = sortAgentSessions(sender?.sessions ?? []);
  const query = targetQuery.value.trim().toLowerCase();
  return query ? rows.filter((row) => row.title.toLowerCase().includes(query)) : rows;
});
const activeTargetRow = computed(() => Math.min(targetIndex.value, Math.max(targetRows.value.length - 1, 0)));

// Each opening starts on the row already chosen, so the arrow keys start where the user already
// is, and without the filter of a menu that is no longer open.
watch(targetOpen, (open) => {
  if (!open) return;
  targetQuery.value = "";
  targetIndex.value = Math.max(
    targetRows.value.findIndex((row) => row.id === sender?.targetId),
    0,
  );
});

watch(targetQuery, () => {
  targetIndex.value = 0;
});
const sendableNotes = computed(() => {
  const pending = props.review.notes.filter(isReviewableNote);
  // An outdated note points at a line that has since changed, so it is held back unless the
  // user says otherwise: that is the same rule the note's own card states.
  return includeOutdated.value ? pending : pending.filter((note) => !note.outdated);
});
const draftCount = computed(() => sendableNotes.value.filter((note) => note.status === "draft").length);
const outdatedCount = computed(() => props.review.notes.filter((note) => note.outdated).length);
const targetSession = computed(() => sender?.sessions.find((session) => session.id === sender.targetId) ?? undefined);
/** A blocked session is not working, so only a real turn makes the send a decision. */
const targetBusy = computed(() => agentAttention(targetSession.value) === "busy");
const showBusyChoice = computed(() => sender?.target === "opencode" && busyChoiceOpen.value && targetBusy.value);
const canSend = computed(() => sendableNotes.value.length > 0);

function sendNow(queue: boolean) {
  busyChoiceOpen.value = false;
  void sender?.send(
    sendableNotes.value.map((note) => note.id),
    queue,
  );
}

/** A working agent cannot be interrupted, so the button asks instead of sending into a turn. */
function requestSend() {
  if (sender?.target === "markdown") {
    sendNow(false);
    return;
  }
  if (targetBusy.value) {
    busyChoiceOpen.value = true;
    return;
  }
  sendNow(false);
}

function chooseTarget(sessionId: string) {
  sender?.selectTarget(sessionId);
  targetOpen.value = false;
}

function chooseReviewTarget(event: Event) {
  const target = (event.currentTarget as HTMLSelectElement).value;
  if (target === "markdown" || target === "opencode") sender?.selectReviewTarget(target);
}

function moveTargetRow(step: number) {
  const total = targetRows.value.length;
  if (total > 0) targetIndex.value = (targetIndex.value + step + total) % total;
}

function chooseActiveTarget() {
  const session = targetRows.value[activeTargetRow.value];
  if (session) chooseTarget(session.id);
}

let diffGeneration = 0;
let mounted = true;

/**
 * Opens a draft, or extends the open one when the click lands on the same side.
 * Extending keeps the range as the span between the first and last clicked line; the
 * composer shows it, and Cancel is the way back to a single-line note.
 */
function openDraft(side: ReviewSide, line: number) {
  draftError.value = "";
  const open = draft.value;
  if (open && open.side === side) {
    draft.value = { ...open, lineStart: Math.min(open.lineStart, line), lineEnd: Math.max(open.lineEnd, line) };
    return;
  }
  draft.value = { side, lineStart: line, lineEnd: line };
}

function cancelDraft() {
  draft.value = null;
  draftError.value = "";
}

function sideName(side: number): ReviewSide {
  return side === 1 ? "old" : "new";
}

async function saveNoteAt(
  side: ReviewSide,
  lineStart: number,
  lineEnd: number | null,
  content: string,
): Promise<boolean> {
  if (props.path === null) return false;
  const range = lineEnd && lineEnd > lineStart ? lineEnd : null;
  return props.review.addNote({
    path: props.path,
    side,
    lineStart,
    lineEnd: range ?? undefined,
    content,
    code: reviewRangeCode(lineTexts.value, side, lineStart, range),
  });
}

async function saveDraft(content: string): Promise<boolean> {
  const target = draft.value;
  if (!target) return false;
  draftError.value = "";
  if (diff.value?.large) {
    for (let line = target.lineStart; line <= target.lineEnd; line += 1) {
      if (!lineTexts.value.has(`${target.side}:${line}`)) {
        draftError.value =
          "Some selected lines are unavailable in the loaded diff pages. Choose a shorter range to include all its code.";
        return false;
      }
    }
  }
  const created = await saveNoteAt(target.side, target.lineStart, target.lineEnd, content);
  if (created) cancelDraft();
  return created;
}

function rowAnchor(line: GitDiffPageLine): { side: ReviewSide; line: number } | null {
  if (line.kind === "hunk") return null;
  if (line.kind === "removed" && line.oldLineNumber) return { side: "old", line: line.oldLineNumber };
  if (line.newLineNumber) return { side: "new", line: line.newLineNumber };
  if (line.oldLineNumber) return { side: "old", line: line.oldLineNumber };
  return null;
}

function isDraftRow(line: GitDiffPageLine): boolean {
  const anchor = rowAnchor(line);
  return Boolean(anchor && draft.value && anchor.side === draft.value.side && anchor.line === draft.value.lineStart);
}

function isDraftSelection(line: GitDiffPageLine | undefined): boolean {
  if (!line) return false;
  const anchor = rowAnchor(line);
  return Boolean(
    anchor &&
    draft.value &&
    anchor.side === draft.value.side &&
    anchor.line >= draft.value.lineStart &&
    anchor.line <= draft.value.lineEnd,
  );
}

function notesForRow(line: GitDiffPageLine): ReviewNote[] {
  const anchor = rowAnchor(line);
  if (!anchor) return [];
  // A range is listed on its first line, which is where the composer opens too.
  return fileNotes.value.filter((note) => note.side === anchor.side && note.lineStart === anchor.line);
}

function errorText(error: unknown): string {
  return isIpcError(error) ? error.message : error instanceof Error ? error.message : String(error);
}

function createHunks(path: string, patch: string) {
  const preamble: string[] = [];
  const sections: Array<{ title: string; patch: string }> = [];
  let current: string[] | null = null;
  let title = "";
  for (const line of patch.split("\n")) {
    if (line.startsWith("@@")) {
      if (current) sections.push({ title, patch: [...preamble, ...current].join("\n") });
      current = [line];
      title = line;
    } else if (current) current.push(line);
    else preamble.push(line);
  }
  if (current) sections.push({ title, patch: [...preamble, ...current].join("\n") });
  return sections.map((section) => {
    const file = new DiffFile(`a/${path}`, "", `b/${path}`, "", [section.patch]);
    file.initTheme("dark");
    file.init();
    file.buildUnifiedDiffLines();
    return { title: section.title, file };
  });
}

async function loadDiff(path: string, preservePosition = false) {
  const request = ++diffGeneration;
  const checkoutId = props.checkout.id;
  const oldScrollTop = preservePosition ? diffScrollTop.value : props.scrollTop;
  // E.3: a different file is a different selection, so the previous diff goes away rather
  // than sitting under the loading state. A refresh of the same file keeps its place.
  const keepPreviousDiff = diff.value !== null && selectedPath.value === path;
  if (selectedPath.value === path) largeDiff.reset();
  selectedPath.value = path;
  if (!keepPreviousDiff) {
    diff.value = null;
    diffHunks.value = [];
    collapsedHunks.value = [];
  }
  diffScrollTop.value = oldScrollTop;
  diffError.value = "";
  diffState.value = "loading";
  try {
    const result = await getGitDiff(checkoutId, path);
    if (
      !mounted ||
      request !== diffGeneration ||
      selectedPath.value !== path ||
      props.checkout.id !== checkoutId ||
      props.path !== path
    )
      return;
    // A refresh that changed nothing must not redraw. The diff view keeps the open note
    // composer in state that a new DiffFile identity wipes, and git reports every write in the
    // workdir, not just in the file on screen: rebuilding on each one closed the composer the
    // moment the user started typing, for a diff that had not moved.
    if (result.patch !== diff.value?.patch) {
      diff.value = result;
      collapsedHunks.value = [];
      if (!result.isBinary && !result.symlinkTarget && result.patch.includes("@@")) {
        diffHunks.value = createHunks(path, result.patch);
      }
    }
    diffState.value = "ready";
    emit("ready", path);
    await nextTick();
    if (request !== diffGeneration || props.checkout.id !== checkoutId || props.path !== path) return;
    if (diffViewport.value) diffViewport.value.scrollTop = oldScrollTop;
    if (result.large && !result.tooLarge && !result.isBinary) loadVisiblePages();
  } catch (error) {
    if (
      !mounted ||
      request !== diffGeneration ||
      selectedPath.value !== path ||
      props.checkout.id !== checkoutId ||
      props.path !== path
    )
      return;
    // A diff that cannot be read leaves the panel with nothing to show, so the reason is
    // drawn in it: a toast would expire and leave an empty panel unexplained.
    diffError.value = errorText(error);
    diffState.value = "error";
  }
}

function toggleFile(path: string) {
  expandedPaths.value = expandedPaths.value.includes(path)
    ? expandedPaths.value.filter((open) => open !== path)
    : [...expandedPaths.value, path];
}

watch(
  () => [props.checkout.id, props.path] as const,
  ([, path]) => {
    if (path !== null) void loadDiff(path);
  },
  { immediate: true, flush: "sync" },
);

// What is open in the change set belongs to the workdir it was opened in.
watch(
  () => props.checkout.id,
  () => {
    expandedPaths.value = [];
  },
);

watch(
  () => [props.checkout.id, props.path] as const,
  () => {
    draft.value = null;
  },
);

// Reports the anchor text of every note whose line the diff renders, so the backend can
// stamp drift. It only fires when a resolved text changes, never on plain scrolling.
watch(
  anchorChecks,
  (checks) => {
    if (checks.length > 0 && props.path !== null) void props.review.verifyAnchors(props.path, checks);
  },
  { deep: true },
);

watch(
  () => props.gitSnapshot.statusRevision,
  async (revision, previous) => {
    if (revision === previous || props.path === null) return;
    if (props.gitSnapshot.checkoutId !== props.checkout.id || !props.gitSnapshot.status) return;
    if (props.gitSnapshot.status.files.some((file) => file.path === props.path)) await loadDiff(props.path, true);
    else {
      diffGeneration += 1;
      diff.value = null;
      diffHunks.value = [];
      diffScrollTop.value = 0;
      diffError.value = "This file is no longer in the current Git changes.";
      diffState.value = "error";
    }
  },
);

function onDiffScroll(event: Event) {
  diffScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
  emit("scrollPositionChanged", diffScrollTop.value);
  if (diff.value?.large) loadVisiblePages();
}

function toggleHunk(index: number) {
  collapsedHunks.value = collapsedHunks.value.includes(index)
    ? collapsedHunks.value.filter((collapsed) => collapsed !== index)
    : [...collapsedHunks.value, index];
  if (diff.value?.large) {
    void nextTick(() => {
      const viewport = diffViewport.value;
      if (!viewport) return;
      const maximum = Math.max(0, largeDiffLineCount.value * DIFF_ROW_HEIGHT - viewport.clientHeight);
      if (viewport.scrollTop > maximum) viewport.scrollTop = maximum;
      diffScrollTop.value = viewport.scrollTop;
      loadVisiblePages();
    });
  }
}

onUnmounted(() => {
  mounted = false;
  diffGeneration += 1;
});
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col" aria-label="File diff">
    <!-- A file of the change-set stack is headed by its own row, so it needs no header here. -->
    <header v-if="!embedded" class="document-toolbar shrink-0 border-b px-3 py-2">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p class="truncate text-[11px] text-(--marvis-text-dim)" :title="title">{{ title }}</p>
          <p v-if="!path && branch" class="truncate text-[11px] text-(--marvis-text-faint)" :title="branch">
            {{ branch }}
          </p>
        </div>
      </div>
      <div v-if="sender" class="mt-2 flex flex-col items-end gap-1">
        <div class="flex flex-wrap items-center justify-end gap-1.5">
          <button
            type="button"
            data-testid="send-review"
            :disabled="!canSend"
            :aria-label="sender.target === 'markdown' ? 'Export as Markdown' : 'Send to opencode'"
            class="marvis-control h-7 justify-center px-2.5 text-xs"
            @click="requestSend"
          >
            {{ sender.target === "markdown" ? "Export as Markdown" : "Send to opencode" }}
          </button>
          <select
            :value="sender.target"
            data-testid="review-target"
            aria-label="Review destination"
            class="marvis-control h-7 px-2 text-xs"
            @change="chooseReviewTarget"
          >
            <option value="markdown">Markdown</option>
            <option value="opencode">OpenCode</option>
          </select>
          <!-- One session is the default target, so the picker only earns its place above one.
               It is the app's own menu rather than a native select, and it earns a search only
               once there are more rows than a short list is worth reading. -->
          <PopoverRoot v-if="sender.target === 'opencode' && sender.sessions.length > 1" v-model:open="targetOpen">
            <PopoverTrigger
              data-testid="send-target"
              aria-label="Send review to"
              :title="targetSession?.title"
              class="text-menu-control max-w-44 gap-1 whitespace-nowrap"
            >
              <span class="min-w-0 truncate">{{ targetSession?.title ?? "Choose a session" }}</span>
              <ChevronDownIcon class="icon-xs shrink-0 text-(--marvis-text-faint)" aria-hidden="true" />
            </PopoverTrigger>
            <!-- Portalled for the same reason the document toolbar portals its own: the list is
                 absolutely positioned and the diff under it paints over anything left in place. -->
            <PopoverPortal>
              <!-- The keys are read here rather than on the search, so the menu answers them the
                   same whether or not a checkout has enough sessions to be searched. -->
              <PopoverContent
                side="bottom"
                align="end"
                :side-offset="4"
                class="surface-popover marvis-menu w-64"
                @keydown.down.prevent="moveTargetRow(1)"
                @keydown.up.prevent="moveTargetRow(-1)"
                @keydown.enter.prevent="chooseActiveTarget"
              >
                <input
                  v-if="searchesTargets"
                  v-model="targetQuery"
                  type="search"
                  aria-label="Search sessions"
                  placeholder="Search…"
                  class="marvis-menu-search min-w-0 appearance-none"
                />
                <div role="listbox" aria-label="Send review to" class="marvis-menu-scroll flex flex-col">
                  <button
                    v-for="(row, index) in targetRows"
                    :key="row.id"
                    type="button"
                    role="option"
                    :aria-selected="row.id === sender.targetId"
                    :title="row.title"
                    class="menu-item select-none text-left"
                    :class="{ 'is-active': index === activeTargetRow }"
                    @click="chooseTarget(row.id)"
                  >
                    <span class="menu-item-label">{{ row.title }}</span>
                    <CheckIcon v-if="row.id === sender.targetId" class="icon-xxs menu-check" aria-hidden="true" />
                  </button>
                  <p v-if="!targetRows.length" class="menu-note">No session matches "{{ targetQuery }}".</p>
                </div>
              </PopoverContent>
            </PopoverPortal>
          </PopoverRoot>
          <span v-if="canSend" data-testid="send-count" class="text-[11px] text-(--marvis-text-faint)">
            {{ sendableNotes.length }} {{ sendableNotes.length === 1 ? "note" : "notes" }}
            <template v-if="draftCount">· {{ draftCount }} {{ draftCount === 1 ? "draft" : "drafts" }}</template>
          </span>
        </div>
        <div v-if="showBusyChoice" data-testid="send-busy" class="flex flex-col items-end gap-1">
          <p class="max-w-prose text-right text-[11px] text-(--marvis-text-secondary)">
            “{{ targetSession?.title }}” is mid-task. Sending now lands inside its current turn; queueing waits for it
            to finish. This OpenCode version cannot cancel a turn.
          </p>
          <div class="flex items-center gap-1.5">
            <button
              type="button"
              data-testid="send-now"
              class="marvis-control h-6 justify-center px-2 text-xs"
              @click="sendNow(false)"
            >
              Send now
            </button>
            <button
              type="button"
              data-testid="send-queue"
              class="marvis-control h-6 justify-center px-2 text-xs"
              @click="sendNow(true)"
            >
              Queue
            </button>
            <button
              type="button"
              data-testid="send-not-now"
              class="px-2 text-xs text-(--marvis-text-faint) hover:text-(--marvis-text)"
              @click="busyChoiceOpen = false"
            >
              Not now
            </button>
          </div>
        </div>
        <p
          v-if="sender.target === 'opencode' && sender.unfinishedRounds > 0"
          data-testid="unfinished-rounds"
          class="text-[11px] text-(--marvis-text-faint)"
        >
          {{ sender.unfinishedRounds }} {{ sender.unfinishedRounds === 1 ? "round" : "rounds" }} not finished
        </p>
        <label v-if="outdatedCount > 0" class="flex items-center gap-1.5 text-[11px] text-(--marvis-text-faint)">
          <input
            v-model="includeOutdated"
            type="checkbox"
            data-testid="include-outdated"
            class="accent-(--marvis-accent)"
          />
          Include {{ outdatedCount }} outdated {{ outdatedCount === 1 ? "note" : "notes" }}
        </label>
      </div>
    </header>
    <template v-if="path === null">
      <div class="diff-files min-h-0 flex-1 overflow-auto">
        <p v-if="gitSnapshot.statusState === 'loading'" role="status" class="pane-state text-sm">Loading changes…</p>
        <p v-else-if="gitSnapshot.statusState === 'error'" role="alert" class="pane-state text-sm">
          {{ gitSnapshot.changesStatusError || gitSnapshot.statusError }}
        </p>
        <p v-else-if="changedFiles.length === 0" role="status" class="pane-state text-sm">No changed files.</p>
        <template v-else>
          <section v-for="file in changedFiles" :key="file.path" class="diff-file">
            <button
              type="button"
              class="diff-file-header"
              :aria-expanded="expandedPaths.includes(file.path)"
              :title="file.oldPath ? `${file.oldPath} → ${file.path}` : file.path"
              @click="toggleFile(file.path)"
            >
              <span class="diff-status" :data-status="file.status">{{ file.status }}</span>
              <span class="truncate" :title="file.path">{{ file.path }}</span>
              <span v-if="file.additions" class="diff-add">+{{ file.additions }}</span>
              <span v-if="file.deletions" class="diff-del">-{{ file.deletions }}</span>
            </button>
            <!-- Git pages one path at a time, so the whole change set is a stack of single-file
                 diffs rather than one merged patch. Each file is diffed when it is opened. -->
            <FileDiff
              v-if="expandedPaths.includes(file.path)"
              :checkout="checkout"
              :git-snapshot="gitSnapshot"
              :review="review"
              :path="file.path"
              :scroll-top="0"
              embedded
            />
          </section>
        </template>
      </div>
    </template>
    <template v-else>
      <p v-if="diffState === 'loading' && !diff" role="status" class="pane-state text-sm">Loading diff…</p>
      <!-- A diff that will not load leaves nothing else to show, so the reason is drawn here
           rather than left to a toast that expires over an empty panel. -->
      <p v-else-if="diffState === 'error'" role="alert" class="pane-state text-sm">
        {{ diffError || "This diff could not be read." }}
      </p>
      <!-- A state of the file itself, not a failure: the panel's whole content is the reason,
           so it stays drawn here rather than expiring in a toast. Same for the three below. -->
      <p v-else-if="diff?.isBinary" role="status" class="pane-state text-sm">Binary file; text diff is unavailable.</p>
      <p v-else-if="diff?.symlinkTarget !== undefined" role="status" class="pane-state text-sm">
        Symlink target: <code class="break-all text-(--marvis-text)">{{ diff.symlinkTarget }}</code>
      </p>
      <p v-else-if="diff?.tooLarge" role="status" class="pane-state text-sm">
        This diff exceeds safe preview limits (100,000 lines, 10,000 hunks, 32 MiB, 4 KiB hunk headers, or 64 KiB per
        line). Reduce the change size to view it.
      </p>
      <p v-else-if="showNoTextHunks" role="status" class="pane-state text-sm">
        No text hunks are available for this change.
      </p>
      <template v-else-if="(diffState === 'ready' || diffState === 'loading') && diff">
        <p v-if="diff.large" class="shrink-0 px-3 py-1 text-[10px] text-(--marvis-text-faint)">
          {{ diff.totalLines.toLocaleString() }} diff rows · virtualized view · click + note on two lines to comment on
          a range
        </p>
        <p v-else class="shrink-0 px-3 py-1 text-[10px] text-(--marvis-text-faint)">
          Drag across line numbers to select a range, then click + note on its last line.
        </p>
        <div
          ref="diffViewport"
          class="diff-viewport min-h-0 flex-1 overflow-auto font-mono text-[12px]"
          aria-label="Diff contents"
          @scroll="onDiffScroll"
        >
          <template v-if="diff.large && !diff.tooLarge">
            <div
              :style="{
                paddingTop: `${visibleLargeDiffWindow.paddingTop}px`,
                paddingBottom: `${visibleLargeDiffWindow.paddingBottom}px`,
              }"
            >
              <template v-for="row in visibleLargeDiffWindow.rows" :key="row.visualIndex">
                <div
                  data-testid="large-diff-row"
                  class="group flex h-6 min-w-max items-center overflow-hidden whitespace-pre text-[12px]"
                  :class="{ 'review-range-selected': isDraftSelection(row.line) }"
                >
                  <button
                    v-if="row.line?.kind === 'hunk'"
                    type="button"
                    class="diff-hunk h-full w-full truncate px-2 text-left text-(--marvis-accent)"
                    :aria-expanded="!row.collapsed"
                    :title="row.line.text"
                    @click="toggleHunk(row.hunkIndex)"
                  >
                    {{ row.collapsed ? "▸" : "▾" }} {{ row.line.text }}
                  </button>
                  <template v-else-if="row.line">
                    <span class="w-12 shrink-0 select-none pr-2 text-right text-(--marvis-text-faint)">{{
                      row.line.oldLineNumber ?? ""
                    }}</span>
                    <span class="w-12 shrink-0 select-none pr-2 text-right text-(--marvis-text-faint)">{{
                      row.line.newLineNumber ?? ""
                    }}</span>
                    <span
                      class="w-4 shrink-0 text-center"
                      :class="
                        row.line.kind === 'added'
                          ? 'text-(--marvis-green)'
                          : row.line.kind === 'removed'
                            ? 'text-(--marvis-red)'
                            : 'text-(--marvis-text-faint)'
                      "
                      >{{ row.line.text[0] }}</span
                    >
                    <code
                      class="pr-4"
                      :class="
                        row.line.kind === 'added'
                          ? 'text-(--marvis-green)'
                          : row.line.kind === 'removed'
                            ? 'text-(--marvis-red)'
                            : 'text-(--marvis-text-secondary)'
                      "
                      >{{ row.line.text.slice(1) }}</code
                    >
                    <button
                      v-if="rowAnchor(row.line)"
                      type="button"
                      class="ml-auto shrink-0 px-2 text-[11px] text-(--marvis-accent) opacity-0 group-hover:opacity-100"
                      :aria-label="`Add review note on line ${rowAnchor(row.line)!.line}`"
                      @click="openDraft(rowAnchor(row.line)!.side, rowAnchor(row.line)!.line)"
                    >
                      + note
                    </button>
                  </template>
                  <span v-else class="px-2 text-(--marvis-text-faint)">{{ row.error || "Loading diff page…" }}</span>
                </div>
                <template v-if="row.line">
                  <ReviewComposer
                    v-if="isDraftRow(row.line)"
                    :side="rowAnchor(row.line)!.side"
                    :line="rowAnchor(row.line)!.line"
                    :line-end="draft?.lineEnd && draft.lineEnd !== draft.lineStart ? draft.lineEnd : null"
                    :code="
                      reviewRangeCode(
                        lineTexts,
                        rowAnchor(row.line)!.side,
                        rowAnchor(row.line)!.line,
                        draft?.lineEnd ?? null,
                      )
                    "
                    :error="draftError"
                    @submit="saveDraft"
                    @cancel="cancelDraft"
                  />
                  <ReviewNoteList
                    v-if="notesForRow(row.line).length > 0"
                    :notes="notesForRow(row.line)"
                    :outcomes="anchorOutcomes"
                    @update-note="review.updateNote"
                    @delete-note="review.deleteNote"
                    @clear-outdated="review.clearOutdated"
                    @resolve-note="review.resolveNote"
                  />
                </template>
              </template>
            </div>
          </template>
          <div v-else class="min-w-max space-y-1 p-1">
            <section v-for="(hunk, index) in diffHunks" :key="`${path}-${index}`" class="min-w-0">
              <button
                type="button"
                class="diff-hunk mb-1 w-full truncate border-b border-(--marvis-border) px-2 py-1 text-left font-mono text-[10px] text-(--marvis-text-dim)"
                :aria-expanded="!collapsedHunks.includes(index)"
                :title="hunk.title"
                @click="toggleHunk(index)"
              >
                {{ collapsedHunks.includes(index) ? "▸" : "▾" }} {{ hunk.title }}
              </button>
              <DiffViewWithMultiSelect
                v-if="!collapsedHunks.includes(index)"
                :diff-file="hunk.file"
                :diff-view-mode="DiffModeEnum.Unified"
                diff-view-theme="dark"
                :diff-view-add-widget="true"
                :extend-data="extendData"
                :diff-view-highlight="true"
                :diff-view-font-size="13"
                class="min-w-0"
              >
                <template #widget="{ lineNumber, fromLineNumber, side, onClose }">
                  <ReviewComposer
                    :side="sideName(side)"
                    :line="fromLineNumber"
                    :line-end="lineNumber !== fromLineNumber ? lineNumber : null"
                    :code="reviewRangeCode(lineTexts, sideName(side), fromLineNumber, lineNumber)"
                    @submit="
                      async (content: string) => {
                        if (await saveNoteAt(sideName(side), fromLineNumber, lineNumber, content)) onClose();
                      }
                    "
                    @cancel="onClose"
                  />
                </template>
                <template #extend="{ data }">
                  <ReviewNoteList
                    :notes="data"
                    :outcomes="anchorOutcomes"
                    @update-note="review.updateNote"
                    @delete-note="review.deleteNote"
                    @clear-outdated="review.clearOutdated"
                    @resolve-note="review.resolveNote"
                  />
                </template>
              </DiffViewWithMultiSelect>
            </section>
          </div>
        </div>
        <div v-if="notesWithoutLine.length > 0" class="shrink-0 border-t border-(--marvis-border)">
          <p class="px-2 py-1 text-[10px] text-(--marvis-text-faint)">
            {{ notesWithoutLine.length }}
            {{ notesWithoutLine.length === 1 ? "note points" : "notes point" }} at a line that is no longer in this diff
          </p>
          <ReviewNoteList
            :notes="notesWithoutLine"
            :outcomes="anchorOutcomes"
            @update-note="review.updateNote"
            @delete-note="review.deleteNote"
            @clear-outdated="review.clearOutdated"
            @resolve-note="review.resolveNote"
          />
        </div>
      </template>
    </template>
  </section>
</template>

<style scoped>
/**
 * The diff library paints itself with `--diff-*` custom properties on `.diff-style-root`, one
 * pair per kind of line. That is the only seam it offers, so F.5 repaints those with the
 * marvis palette instead of replacing its renderer. The selector is deliberately longer than
 * the library's own `[data-theme="dark"]` rules: same-specificity rules would be settled by
 * stylesheet order, which is not something a component can rely on.
 */
.diff-viewport :deep(.diff-tailwindcss-wrapper[data-theme="dark"] .diff-style-root) {
  --diff-border--: var(--marvis-border);
  --diff-plain-content--: var(--marvis-bg-0);
  --diff-plain-lineNumber--: var(--marvis-bg-0);
  --diff-expand-content--: var(--marvis-bg-1);
  --diff-expand-lineNumber--: var(--marvis-bg-1);
  --diff-empty-content--: var(--marvis-bg-0);
  --diff-plain-lineNumber-color--: var(--marvis-text-faint);
  --diff-expand-lineNumber-color--: var(--marvis-text-faint);
  /* An added or removed line is the diff's green and red, the same ones the stats use. */
  --diff-add-content--: color-mix(in srgb, var(--marvis-green) 14%, var(--marvis-bg-0));
  --diff-del-content--: color-mix(in srgb, var(--marvis-red) 14%, var(--marvis-bg-0));
  --diff-add-lineNumber--: color-mix(in srgb, var(--marvis-green) 22%, var(--marvis-bg-0));
  --diff-del-lineNumber--: color-mix(in srgb, var(--marvis-red) 22%, var(--marvis-bg-0));
  --diff-add-content-highlight--: color-mix(in srgb, var(--marvis-green) 24%, var(--marvis-bg-0));
  --diff-del-content-highlight--: color-mix(in srgb, var(--marvis-red) 24%, var(--marvis-bg-0));
  --diff-hunk-content--: var(--marvis-bg-1);
  --diff-hunk-lineNumber--: var(--marvis-bg-1);
  --diff-hunk-lineNumber-hover--: var(--marvis-accent);
  --diff-hunk-content-color--: var(--marvis-text-secondary);
  --diff-add-widget--: var(--marvis-accent);
  --diff-add-widget-color--: var(--marvis-text);
  --diff-multi-select-bg: var(--marvis-accent);
  --diff-multi-select-border: var(--marvis-accent);
}

.diff-viewport :deep(.diff-tailwindcss-wrapper[data-theme="dark"] [data-state="diff"]),
.diff-viewport :deep(.diff-tailwindcss-wrapper[data-theme="dark"] [data-state="plain"]),
.diff-viewport :deep(.diff-tailwindcss-wrapper[data-theme="dark"] [data-state="hunk"]) {
  color: var(--marvis-text);
}

/* Hunk headers sit on the change's own surface, as the mockup's group rows do. */
.diff-hunk {
  background: var(--marvis-bg-1);
  color: var(--marvis-text-secondary);
}

.diff-hunk:hover {
  background: var(--marvis-bg-2);
  color: var(--marvis-text);
}

/* One row per changed file in the whole change set, and its diff under it. */
.diff-file + .diff-file {
  border-top: 1px solid var(--marvis-border);
}

.diff-file-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-height: 24px;
  padding: 4px 6px;
  border: none;
  background: transparent;
  color: var(--marvis-text);
  font-family: inherit;
  font-size: 12px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  cursor: pointer;
}

.diff-file-header:hover {
  background: var(--marvis-bg-2);
}

.review-range-selected {
  background: color-mix(in srgb, var(--marvis-accent) 16%, var(--marvis-bg-0));
}

.diff-status {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--marvis-text-secondary);
}

.diff-status[data-status="A"] {
  color: var(--marvis-green);
}

.diff-status[data-status="D"] {
  color: var(--marvis-red);
}

.diff-status[data-status="U"] {
  color: var(--marvis-text-faint);
}

/* A file inside the change-set stack scrolls on its own, so the virtual window of a large
   diff has a container to follow. */
.diff-file :deep(.diff-viewport) {
  max-height: 60vh;
}
</style>
