<script setup lang="ts">
/* eslint-disable vue/html-indent, vue/html-closing-bracket-newline */
import { DiffFile, DiffModeEnum, DiffView } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { computed, nextTick, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff, GitDiffPageLine } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import { buildDiffLineTexts, diffLineText, reviewRangeCode } from "../domain/review";
import type { AnchorOutcome, ReviewNote, ReviewSide } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";
import type { ReviewAnchorCheck } from "../lib/ipc";
import { getGitDiff } from "../lib/ipc";
import { DIFF_ROW_HEIGHT, useLargeDiff } from "./use-large-diff";
import ReviewComposer from "./ReviewComposer.vue";
import ReviewNoteList from "./ReviewNoteList.vue";

const props = defineProps<{
  checkout: Checkout;
  gitSnapshot: ActiveGitSnapshot;
  review: Pick<
    ActiveReviewNotes,
    "notes" | "addNote" | "updateNote" | "deleteNote" | "verifyAnchors" | "clearOutdated" | "resolveNote"
  >;
  path: string;
  active: boolean;
  scrollTop: number;
}>();
const emit = defineEmits<{
  ready: [path: string];
  scrollPositionChanged: [top: number];
}>();

const diff = shallowRef<GitFileDiff | null>(null);
const diffHunks = shallowRef<Array<{ title: string; file: DiffFile }>>([]);
const collapsedHunks = ref<number[]>([]);
const diffState = ref<"idle" | "loading" | "ready" | "error">("idle");
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
const largeDiff = useLargeDiff(() => props.checkout.id, selectedPath, diff, collapsedHunks, diffScrollTop);
const { diffPageError, largeDiffLineCount, loadVisiblePages, visibleLargeDiffWindow } = largeDiff;
const draft = ref<{ side: ReviewSide; lineStart: number; lineEnd: number } | null>(null);
/** Diff texts keyed by `${side}:${line}`, for the whole file or for the rendered window. */
const lineTexts = computed(() => {
  if (!diff.value?.large || diff.value.tooLarge) return buildDiffLineTexts(diff.value?.patch ?? "");
  const texts = new Map<string, string>();
  for (const row of visibleLargeDiffWindow.value.rows) {
    if (!row.line) continue;
    const anchor = rowAnchor(row.line);
    if (anchor) texts.set(`${anchor.side}:${anchor.line}`, row.line.text.slice(1));
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
let diffGeneration = 0;
let mounted = true;
let markedViewedKey: string | null = null;

function lineCode(side: ReviewSide, line: number): string {
  return diffLineText(lineTexts.value, side, line);
}

/**
 * Opens a draft, or extends the open one when the click lands on the same side.
 * Extending keeps the range as the span between the first and last clicked line; the
 * composer shows it, and Cancel is the way back to a single-line note.
 */
function openDraft(side: ReviewSide, line: number) {
  const open = draft.value;
  if (open && open.side === side) {
    draft.value = { ...open, lineStart: Math.min(open.lineStart, line), lineEnd: Math.max(open.lineEnd, line) };
    return;
  }
  draft.value = { side, lineStart: line, lineEnd: line };
}

function cancelDraft() {
  draft.value = null;
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
  const keepPreviousDiff = diff.value !== null;
  if (selectedPath.value === path) largeDiff.reset();
  selectedPath.value = path;
  if (!keepPreviousDiff) {
    diff.value = null;
    diffHunks.value = [];
  }
  collapsedHunks.value = [];
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
    diff.value = result;
    if (!result.isBinary && !result.symlinkTarget && result.patch.includes("@@")) {
      diffHunks.value = createHunks(path, result.patch);
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
    diffError.value = errorText(error);
    diffState.value = "error";
  }
}

watch(
  () => [props.checkout.id, props.path] as const,
  ([, path]) => void loadDiff(path),
  { immediate: true, flush: "sync" },
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
    if (checks.length > 0) void props.review.verifyAnchors(props.path, checks);
  },
  { deep: true },
);

watch(
  () => props.gitSnapshot.statusRevision,
  async (revision, previous) => {
    if (revision === previous || props.gitSnapshot.checkoutId !== props.checkout.id || !props.gitSnapshot.status)
      return;
    if (props.gitSnapshot.status.files.some((file) => file.path === props.path)) await loadDiff(props.path, true);
    else {
      diffGeneration += 1;
      diff.value = null;
      diffHunks.value = [];
      diffScrollTop.value = 0;
      diffState.value = "error";
      diffError.value = "This file is no longer in the current Git changes.";
    }
  },
);

watch(
  [() => props.active, diffState, diff, visibleLargeDiffWindow],
  async ([active, state, currentDiff, largeWindow]) => {
    if (!active || state !== "ready" || !currentDiff) return;
    const canMark =
      !currentDiff.tooLarge &&
      !currentDiff.isBinary &&
      currentDiff.symlinkTarget === undefined &&
      (currentDiff.large ? largeWindow.rows.some((row) => row.line) : hasTextHunks.value);
    if (!canMark) return;
    const checkoutId = props.checkout.id;
    const path = props.path;
    const request = diffGeneration;
    const key = `${checkoutId}:${path}:${request}`;
    if (markedViewedKey === key) return;
    await nextTick();
    if (
      !mounted ||
      !props.active ||
      request !== diffGeneration ||
      props.checkout.id !== checkoutId ||
      props.path !== path
    )
      return;
    markedViewedKey = key;
    void props.gitSnapshot.markViewed(checkoutId, path);
  },
  { flush: "post" },
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
  <section class="flex min-h-0 flex-1 flex-col overflow-hidden" aria-label="File diff">
    <p v-if="diffState === 'loading' && !diff" role="status" class="text-xs text-zinc-500">Loading diff…</p>
    <p v-else-if="diffState === 'error'" role="alert" class="text-xs text-red-300">{{ diffError }}</p>
    <p v-else-if="diff?.isBinary" role="status" class="text-xs text-amber-300">
      Binary file; text diff is unavailable.
    </p>
    <p v-else-if="diff?.symlinkTarget !== undefined" role="status" class="text-xs text-zinc-400">
      Symlink target: <code class="break-all text-zinc-200">{{ diff.symlinkTarget }}</code>
    </p>
    <p v-else-if="diff?.tooLarge" role="status" class="text-xs text-amber-300">
      This diff exceeds safe preview limits (100,000 lines, 10,000 hunks, 32 MiB, 4 KiB hunk headers, or 64 KiB per
      line). Reduce the change size to view it.
    </p>
    <p v-else-if="showNoTextHunks" role="status" class="text-xs text-zinc-500">
      No text hunks are available for this change.
    </p>
    <template v-else-if="(diffState === 'ready' || diffState === 'loading') && diff">
      <p v-if="diff.large" class="mb-2 shrink-0 text-[10px] text-zinc-500">
        {{ diff.totalLines.toLocaleString() }} diff rows · virtualized view · all rows available by scrolling
      </p>
      <p v-if="diffPageError" role="alert" class="mb-2 shrink-0 text-xs text-red-300">{{ diffPageError }}</p>
      <div
        ref="diffViewport"
        class="diff-viewport min-h-0 flex-1 overflow-auto border-t border-white/8 font-mono text-[12px]"
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
              >
                <button
                  v-if="row.line?.kind === 'hunk'"
                  type="button"
                  class="h-full w-full truncate bg-white/4 px-2 text-left text-sky-300 hover:bg-white/8"
                  :aria-expanded="!row.collapsed"
                  @click="toggleHunk(row.hunkIndex)"
                >
                  {{ row.collapsed ? "▸" : "▾" }} {{ row.line.text }}
                </button>
                <template v-else-if="row.line">
                  <span class="w-12 shrink-0 select-none pr-2 text-right text-zinc-600">{{
                    row.line.oldLineNumber ?? ""
                  }}</span>
                  <span class="w-12 shrink-0 select-none pr-2 text-right text-zinc-600">{{
                    row.line.newLineNumber ?? ""
                  }}</span>
                  <span
                    class="w-4 shrink-0 text-center"
                    :class="
                      row.line.kind === 'added'
                        ? 'text-green-400'
                        : row.line.kind === 'removed'
                          ? 'text-red-400'
                          : 'text-zinc-600'
                    "
                    >{{ row.line.text[0] }}</span
                  >
                  <code
                    class="pr-4"
                    :class="
                      row.line.kind === 'added'
                        ? 'text-green-200'
                        : row.line.kind === 'removed'
                          ? 'text-red-200'
                          : 'text-zinc-300'
                    "
                    >{{ row.line.text.slice(1) }}</code
                  >
                  <button
                    v-if="rowAnchor(row.line)"
                    type="button"
                    class="ml-auto shrink-0 px-2 text-[11px] text-sky-300 opacity-0 group-hover:opacity-100"
                    :aria-label="`Add review note on line ${rowAnchor(row.line)!.line}`"
                    @click="openDraft(rowAnchor(row.line)!.side, rowAnchor(row.line)!.line)"
                  >
                    + note
                  </button>
                </template>
                <span v-else class="px-2 text-zinc-600">{{ row.error || "Loading diff page…" }}</span>
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
              class="mb-1 w-full truncate border-b border-white/6 bg-white/4 px-2 py-1 text-left font-mono text-[10px] text-zinc-400 hover:bg-white/8"
              :aria-expanded="!collapsedHunks.includes(index)"
              @click="toggleHunk(index)"
            >
              {{ collapsedHunks.includes(index) ? "▸" : "▾" }} {{ hunk.title }}
            </button>
            <DiffView
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
              <template #widget="{ lineNumber, side, onClose }">
                <ReviewComposer
                  :side="sideName(side)"
                  :line="lineNumber"
                  :code="lineCode(sideName(side), lineNumber)"
                  @submit="
                    async (content: string) => {
                      if (await saveNoteAt(sideName(side), lineNumber, null, content)) onClose();
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
            </DiffView>
          </section>
        </div>
      </div>
      <div v-if="notesWithoutLine.length > 0" class="shrink-0 border-t border-white/8">
        <p class="px-2 py-1 text-[10px] text-zinc-500">
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
  </section>
</template>
