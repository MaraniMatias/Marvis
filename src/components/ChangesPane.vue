<script setup lang="ts">
import { DiffFile, DiffModeEnum, DiffView } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { computed, nextTick, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { getGitDiff } from "../lib/ipc";
import { DIFF_ROW_HEIGHT, useLargeDiff } from "./use-large-diff";

const props = defineProps<{ checkout: Checkout; gitSnapshot: ActiveGitSnapshot }>();

const status = computed(() => props.gitSnapshot.status);
const statusState = computed(() => props.gitSnapshot.statusState);
const statusError = computed(() => props.gitSnapshot.changesStatusError);
const watchError = computed(() => props.gitSnapshot.changesWatchError);
const viewedPaths = computed(() => props.gitSnapshot.viewedPaths);
const viewedError = computed(() => props.gitSnapshot.viewedError);
const selectedPath = ref<string | null>(null);
const diff = ref<GitFileDiff | null>(null);
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
const fileScrollTop = ref(0);
const diffViewport = ref<HTMLElement | null>(null);
const diffScrollTop = ref(0);
const largeDiff = useLargeDiff(() => props.checkout.id, selectedPath, diff, collapsedHunks, diffScrollTop);
const { diffPageError, largeDiffLineCount, visibleLargeDiffWindow } = largeDiff;
const visibleFileWindow = computed(() => {
  const files = status.value?.files ?? [];
  const rowHeight = 28;
  const windowSize = 60;
  const start = Math.min(
    Math.max(0, files.length - windowSize),
    Math.max(0, Math.floor(fileScrollTop.value / rowHeight) - 8),
  );
  const end = Math.min(files.length, start + windowSize);
  return {
    files: files.slice(start, end),
    paddingTop: start * rowHeight,
    paddingBottom: (files.length - end) * rowHeight,
  };
});
const viewedCount = computed(() => {
  const viewed = new Set(viewedPaths.value);
  return status.value?.files.filter((file) => viewed.has(file.path)).length ?? 0;
});
let diffGeneration = 0;
let mounted = true;

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

async function markViewed(path: string) {
  await props.gitSnapshot.markViewed(props.checkout.id, path);
}

async function loadDiff(path: string, preservePosition = false) {
  const request = ++diffGeneration;
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
    const result = await getGitDiff(props.checkout.id, path);
    if (!mounted || request !== diffGeneration || selectedPath.value !== path) return;
    diff.value = result;
    if (!result.isBinary && !result.symlinkTarget && result.patch.includes("@@")) {
      diffHunks.value = createHunks(path, result.patch);
    }
    diffState.value = "ready";
    await nextTick();
    if (diffViewport.value) diffViewport.value.scrollTop = oldScrollTop;
    if (result.large && !result.tooLarge && !result.isBinary) void largeDiff.loadVisiblePages();
    if (!result.tooLarge && !result.isBinary && (result.large ? result.totalLines > 0 : result.patch.includes("@@"))) {
      void markViewed(path);
    }
  } catch (error) {
    if (!mounted || request !== diffGeneration || selectedPath.value !== path) return;
    diffError.value = errorText(error);
    diffState.value = "error";
  }
}

function onLargeDiffScroll(event: Event) {
  diffScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
  largeDiff.loadVisiblePages();
}

watch(
  () => props.gitSnapshot.statusRevision,
  async (revision, previous) => {
    if (revision === previous || props.gitSnapshot.checkoutId !== props.checkout.id || !props.gitSnapshot.status) {
      return;
    }
    const result = props.gitSnapshot.status;
    if (selectedPath.value) {
      if (result.files.some((file) => file.path === selectedPath.value)) {
        await loadDiff(selectedPath.value, true);
      } else {
        selectedPath.value = null;
        diff.value = null;
        diffHunks.value = [];
        diffScrollTop.value = 0;
        diffState.value = "idle";
      }
    }
  },
);

watch(
  () => props.checkout.id,
  () => {
    diffGeneration += 1;
    selectedPath.value = null;
    diff.value = null;
    diffHunks.value = [];
    collapsedHunks.value = [];
    diffError.value = "";
    diffScrollTop.value = 0;
    diffState.value = "idle";
  },
  { flush: "sync" },
);

function onFileListScroll(event: Event) {
  fileScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
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
      largeDiff.loadVisiblePages();
    });
  }
}

onUnmounted(() => {
  mounted = false;
  diffGeneration += 1;
});
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="border-b border-white/8 px-4 py-3">
      <p class="truncate text-xs font-medium text-zinc-200">
        {{ status?.branch || (status?.head ? `HEAD ${status.head}` : "Git changes") }}
      </p>
      <p class="mt-1 text-[11px] text-zinc-500">
        <template v-if="status">vs {{ status.defaultBranch }} · {{ status.files.length }} changed</template>
        <template v-else>Comparing with the default branch</template>
      </p>
      <p v-if="status" class="mt-1 text-[11px] text-zinc-500">
        {{ status.aheadCount }} {{ status.aheadCount === 1 ? "commit" : "commits" }} ahead · {{ viewedCount }}/{{
          status.files.length
        }}
        viewed
      </p>
      <p v-if="viewedError" role="alert" class="mt-1 text-[10px] text-amber-300">Viewed progress: {{ viewedError }}</p>
      <p v-if="watchError" role="alert" class="mt-1 text-[11px] text-amber-300">
        Live updates unavailable: {{ watchError }}
      </p>
    </div>

    <div
      class="min-h-0 flex-[0_0_38%] overflow-auto border-b border-white/8 p-2"
      aria-label="Changed files"
      @scroll="onFileListScroll"
    >
      <p v-if="statusState === 'loading'" role="status" class="px-2 py-3 text-xs text-zinc-500">Loading Git status…</p>
      <p v-else-if="statusState === 'error'" role="alert" class="px-2 py-3 text-xs text-red-300">
        {{ statusError }}
      </p>
      <p v-else-if="status?.files.length === 0" role="status" class="px-2 py-3 text-xs text-zinc-500">
        No changed files.
      </p>
      <div
        v-else-if="status"
        :style="{
          paddingTop: `${visibleFileWindow.paddingTop}px`,
          paddingBottom: `${visibleFileWindow.paddingBottom}px`,
        }"
      >
        <button
          v-for="file in visibleFileWindow.files"
          :key="file.path"
          type="button"
          class="flex h-7 w-full items-center gap-2 rounded px-2 text-left text-xs hover:bg-white/6"
          :class="selectedPath === file.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
          @click="loadDiff(file.path)"
        >
          <span class="w-6 shrink-0 text-[10px] font-semibold text-zinc-500">{{ file.status }}</span>
          <span class="truncate font-mono">{{ file.path }}</span>
          <span v-if="viewedPaths.includes(file.path)" class="ml-auto shrink-0 text-[10px] text-green-400">Viewed</span>
        </button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col p-3" aria-label="File diff">
      <p v-if="!selectedPath" class="text-xs text-zinc-500">Select a changed file to load its diff.</p>
      <template v-else>
        <p class="mb-2 break-all font-mono text-[11px] text-zinc-400">{{ selectedPath }}</p>
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
        <template v-else-if="diffState === 'ready' && diff?.large && !diff.tooLarge">
          <p class="mb-2 text-[10px] text-zinc-500">
            {{ diff.totalLines.toLocaleString() }} diff rows · virtualized view · all rows available by scrolling
          </p>
          <p v-if="diffPageError" role="alert" class="mb-2 text-xs text-red-300">{{ diffPageError }}</p>
          <div
            ref="diffViewport"
            class="min-h-0 flex-1 overflow-auto rounded border border-white/8 bg-[#101217] font-mono text-[11px]"
            aria-label="Large diff"
            @scroll="onLargeDiffScroll"
          >
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
                  <span class="w-12 shrink-0 select-none pr-2 text-right text-zinc-600">
                    {{ row.line.oldLineNumber ?? "" }}
                  </span>
                  <span class="w-12 shrink-0 select-none pr-2 text-right text-zinc-600">
                    {{ row.line.newLineNumber ?? "" }}
                  </span>
                  <span
                    class="w-4 shrink-0 text-center"
                    :class="
                      row.line.kind === 'added'
                        ? 'text-green-400'
                        : row.line.kind === 'removed'
                          ? 'text-red-400'
                          : 'text-zinc-600'
                    "
                  >
                    {{ row.line.text[0] }}
                  </span>
                  <code
                    class="pr-4"
                    :class="
                      row.line.kind === 'added'
                        ? 'text-green-200'
                        : row.line.kind === 'removed'
                          ? 'text-red-200'
                          : 'text-zinc-300'
                    "
                  >
                    {{ row.line.text.slice(1) }}
                  </code>
                </template>
                <span v-else class="px-2 text-zinc-600">{{ row.error || "Loading diff page…" }}</span>
              </div>
            </div>
          </div>
        </template>
        <div v-else-if="diffState === 'ready' && diffHunks.length" class="min-h-0 flex-1 overflow-auto space-y-2">
          <section v-for="(hunk, index) in diffHunks" :key="`${selectedPath}-${index}`" class="min-w-0">
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
      </template>
    </div>
  </div>
</template>
