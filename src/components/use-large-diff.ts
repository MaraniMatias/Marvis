import { computed, onUnmounted, ref, shallowRef, watch } from "vue";
import type { Ref } from "vue";
import type { GitDiffPage, GitDiffPageLine, GitFileDiff } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import { useToasts } from "../presentation/toasts";
import { getGitDiffPage } from "../lib/ipc";

const DIFF_PAGE_SIZE = 32;
export const DIFF_ROW_HEIGHT = 22;
const DIFF_WINDOW_SIZE = 80;
const MAX_CACHED_DIFF_PAGES = 8;
const MAX_CONCURRENT_DIFF_PAGE_REQUESTS = 3;

interface VirtualHunk {
  rawStart: number;
  visualStart: number;
  visualEnd: number;
  index: number;
  collapsed: boolean;
}

interface LargeDiffRow {
  visualIndex: number;
  rawIndex: number;
  hunkIndex: number;
  collapsed: boolean;
  line?: GitDiffPageLine;
  error?: string;
}

function errorText(error: unknown): string {
  return isIpcError(error) ? error.message : error instanceof Error ? error.message : String(error);
}

function segmentAt(line: number, segments: VirtualHunk[]): VirtualHunk | undefined {
  let low = 0;
  let high = segments.length - 1;
  while (low <= high) {
    const middle = (low + high) >>> 1;
    const segment = segments[middle];
    if (line < segment.visualStart) high = middle - 1;
    else if (line >= segment.visualEnd) low = middle + 1;
    else return segment;
  }
  return undefined;
}

export function useLargeDiff(
  checkoutId: () => string,
  selectedPath: Ref<string | null>,
  diff: Ref<GitFileDiff | null>,
  collapsedHunks: Ref<number[]>,
  scrollTop: Ref<number>,
) {
  const diffPages = shallowRef<Record<number, GitDiffPage>>({});
  // Keyed by page: the row that could not load says so in place, so a repeated failure on
  // every scroll does not become a toast wall.
  const diffPageErrors = ref<Record<number, string>>({});
  const pending = new Set<string>();
  const { push: pushToast } = useToasts();
  let pageUseOrder: number[] = [];
  let generation = 0;
  let mounted = true;

  const virtualHunks = computed<VirtualHunk[]>(() => {
    let visualLine = 0;
    return (diff.value?.hunks ?? []).map((hunk, index) => {
      const collapsed = collapsedHunks.value.includes(index);
      const rawLength = Math.max(1, hunk.endLine - hunk.startLine);
      const visualLength = collapsed ? 1 : rawLength;
      const segment = {
        rawStart: hunk.startLine,
        visualStart: visualLine,
        visualEnd: visualLine + visualLength,
        index,
        collapsed,
      };
      visualLine += visualLength;
      return segment;
    });
  });

  const largeDiffLineCount = computed(() => virtualHunks.value.at(-1)?.visualEnd ?? 0);
  const visibleLargeDiffWindow = computed(() => {
    const total = largeDiffLineCount.value;
    const maximumStart = Math.max(0, total - DIFF_WINDOW_SIZE);
    const start = Math.min(maximumStart, Math.max(0, Math.floor(scrollTop.value / DIFF_ROW_HEIGHT) - 10));
    const end = Math.min(total, start + DIFF_WINDOW_SIZE);
    const rows: LargeDiffRow[] = [];
    for (let visualIndex = start; visualIndex < end; visualIndex += 1) {
      const segment = segmentAt(visualIndex, virtualHunks.value);
      if (!segment) continue;
      const rawIndex = segment.rawStart + (segment.collapsed ? 0 : visualIndex - segment.visualStart);
      const pageOffset = Math.floor(rawIndex / DIFF_PAGE_SIZE) * DIFF_PAGE_SIZE;
      const page = diffPages.value[pageOffset];
      rows.push({
        visualIndex,
        rawIndex,
        hunkIndex: segment.index,
        collapsed: segment.collapsed,
        line: page?.lines[rawIndex - page.startLine],
        error: diffPageErrors.value[pageOffset],
      });
    }
    return {
      rows,
      paddingTop: start * DIFF_ROW_HEIGHT,
      paddingBottom: (total - end) * DIFF_ROW_HEIGHT,
    };
  });

  function reset() {
    generation += 1;
    diffPages.value = {};
    diffPageErrors.value = {};
    pageUseOrder = [];
  }

  function isCurrent(request: number, path: string, requestCheckoutId: string): boolean {
    return mounted && request === generation && selectedPath.value === path && checkoutId() === requestCheckoutId;
  }

  async function loadPage(path: string, offset: number, request: number, requestCheckoutId: string) {
    const cacheKey = `${request}:${offset}`;
    if (diffPages.value[offset] || diffPageErrors.value[offset] || pending.has(cacheKey)) return;
    if (pending.size >= MAX_CONCURRENT_DIFF_PAGE_REQUESTS) return;
    pending.add(cacheKey);
    diffPageErrors.value = { ...diffPageErrors.value, [offset]: "" };
    try {
      const result = await getGitDiffPage(requestCheckoutId, path, offset, DIFF_PAGE_SIZE);
      if (!isCurrent(request, path, requestCheckoutId)) return;
      if (result.totalLines !== diff.value?.totalLines || result.lines.length === 0) {
        diffPageErrors.value = {
          ...diffPageErrors.value,
          [offset]: "Diff changed while loading. Refresh Changes and try again.",
        };
        return;
      }
      const pages = { ...diffPages.value, [offset]: result };
      pageUseOrder = [...pageUseOrder.filter((pageOffset) => pageOffset !== offset), offset];
      while (pageUseOrder.length > MAX_CACHED_DIFF_PAGES) {
        const expired = pageUseOrder.shift();
        if (expired !== undefined) delete pages[expired];
      }
      diffPages.value = pages;
    } catch (error) {
      if (isCurrent(request, path, requestCheckoutId)) {
        const message = errorText(error);
        // The row says what failed; the toast announces it once, so scrolling the window
        // over the same broken page does not stack one message per attempt.
        pushToast(message);
        diffPageErrors.value = { ...diffPageErrors.value, [offset]: message };
      }
    } finally {
      pending.delete(cacheKey);
      if (mounted) loadVisiblePages();
    }
  }

  function loadVisiblePages() {
    const path = selectedPath.value;
    const request = generation;
    const requestCheckoutId = checkoutId();
    if (!path || !diff.value?.large || diff.value.tooLarge) return;
    const offsets = new Set(
      visibleLargeDiffWindow.value.rows.map((row) => Math.floor(row.rawIndex / DIFF_PAGE_SIZE) * DIFF_PAGE_SIZE),
    );
    for (const offset of offsets) void loadPage(path, offset, request, requestCheckoutId);
  }

  watch([checkoutId, selectedPath], reset, { flush: "sync" });
  onUnmounted(() => {
    mounted = false;
    reset();
  });

  return { diffPages, largeDiffLineCount, loadVisiblePages, reset, visibleLargeDiffWindow };
}
