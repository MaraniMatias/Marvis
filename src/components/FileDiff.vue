<script setup lang="ts">
/* eslint-disable vue/html-indent, vue/html-closing-bracket-newline */
import { DiffFile, DiffModeEnum, DiffView } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { computed, nextTick, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { getGitDiff } from "../lib/ipc";
import { DIFF_ROW_HEIGHT, useLargeDiff } from "./use-large-diff";

const props = defineProps<{ checkout: Checkout; gitSnapshot: ActiveGitSnapshot; path: string; active: boolean }>();

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
const diffScrollTop = ref(0);
const selectedPath = ref<string | null>(null);
const largeDiff = useLargeDiff(() => props.checkout.id, selectedPath, diff, collapsedHunks, diffScrollTop);
const { diffPageError, largeDiffLineCount, loadVisiblePages, visibleLargeDiffWindow } = largeDiff;
let diffGeneration = 0;
let mounted = true;
let markedViewedKey: string | null = null;

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
  const oldScrollTop = preservePosition ? diffScrollTop.value : 0;
  if (selectedPath.value === path) largeDiff.reset();
  selectedPath.value = path;
  diff.value = null;
  diffHunks.value = [];
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
  <section class="flex min-h-0 flex-1 flex-col overflow-hidden p-4" aria-label="File diff">
    <p v-if="diffState === 'loading'" role="status" class="text-xs text-zinc-500">Loading diff…</p>
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
    <template v-else-if="diffState === 'ready' && diff">
      <p v-if="diff.large" class="mb-2 shrink-0 text-[10px] text-zinc-500">
        {{ diff.totalLines.toLocaleString() }} diff rows · virtualized view · all rows available by scrolling
      </p>
      <p v-if="diffPageError" role="alert" class="mb-2 shrink-0 text-xs text-red-300">{{ diffPageError }}</p>
      <div
        ref="diffViewport"
        class="min-h-0 flex-1 overflow-auto rounded border border-white/8 bg-[#101217] font-mono text-[11px]"
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
            <div
              v-for="row in visibleLargeDiffWindow.rows"
              :key="row.visualIndex"
              data-testid="large-diff-row"
              class="flex h-[22px] min-w-max items-center overflow-hidden whitespace-pre"
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
              </template>
              <span v-else class="px-2 text-zinc-600">{{ row.error || "Loading diff page…" }}</span>
            </div>
          </div>
        </template>
        <div v-else class="min-w-max space-y-2 p-2">
          <section v-for="(hunk, index) in diffHunks" :key="`${path}-${index}`" class="min-w-0">
            <button
              type="button"
              class="mb-1 w-full truncate rounded bg-white/4 px-2 py-1 text-left font-mono text-[10px] text-zinc-400 hover:bg-white/8"
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
              :diff-view-highlight="true"
              :diff-view-font-size="11"
              class="min-w-0"
            />
          </section>
        </div>
      </div>
    </template>
  </section>
</template>
