import { listen } from "@tauri-apps/api/event";
import { onScopeDispose, reactive, toValue, watch } from "vue";
import type { MaybeRefOrGetter } from "vue";
import type { GitDiffStats } from "../domain/git";
import type { Repo } from "../domain/workspace";
import { getGitCheckoutDiffStats, getGitDiffStats } from "../lib/ipc";

/** What one row of the Changes tab shows. A file Git has no count for keeps both absent, so
 *  the row draws no number rather than a zero it did not measure. */
export interface DiffStatsFile {
  additions?: number;
  deletions?: number;
}

export interface DiffStats {
  /** The totals of every registered Git checkout, keyed by checkout id. One call covers the
   *  whole sidebar, which is why no view walks the checkouts one at a time. A checkout Git
   *  cannot answer for is absent, so its row shows nothing rather than a false clean bill. */
  readonly checkoutTotals: Readonly<Record<string, GitDiffStats>>;
  /** The counts of the active checkout's changed files, keyed by path. */
  readonly fileCounts: Readonly<Record<string, DiffStatsFile>>;
}

/**
 * The `+N`/`-N` a workdir row and a Changes row show.
 *
 * The counts are refreshed by the same `git-status-changed` signal that refreshes the file
 * list, and against the same base ref, so a number can never sit on a row describing a
 * different change set than the one the panel lists. The store is shared by every caller: the
 * sidebar and the inspector want the same data, and one listener plus one call per change
 * serves both.
 *
 * The base ref is resolved in Rust, from the checkout's own default branch. Nothing about it
 * crosses the IPC boundary from the WebView, and no path or ref is accepted as input here.
 */
const state = reactive({
  checkoutTotals: {} as Record<string, GitDiffStats>,
  fileCounts: {} as Record<string, DiffStatsFile>,
});

/** The checkout whose files the Changes tab lists. Every caller names the same one. */
let activeCheckoutId: string | null = null;
let consumers = 0;
let unlisten: (() => void) | undefined;
let generation = 0;
let refreshQueued = false;

export function useDiffStats(repos: MaybeRefOrGetter<Repo[]>, checkoutId: MaybeRefOrGetter<string | null>): DiffStats {
  if (++consumers === 1) void startListening();

  // Each caller decides on its own what counts as a change, because the sidebar watches
  // every checkout while the inspector watches only the one it is showing. Two callers
  // asking at once collapse into one call below, so a redundant ask costs nothing. The
  // immediate call always asks: a caller that just mounted is showing rows whose numbers
  // it does not have yet.
  watch(
    [
      // Only the set of checkouts matters here, not the rest of the workspace: a terminal
      // opening in a checkout is not a change to what the sidebar has to ask Git about.
      () =>
        toValue(repos)
          .flatMap((repo) => repo.checkouts)
          .map((checkout) => `${checkout.id}:${checkout.isMissing ? 1 : 0}`)
          .sort()
          .join(","),
      () => toValue(checkoutId),
    ],
    ([, currentCheckoutId]) => {
      if (currentCheckoutId !== activeCheckoutId) {
        activeCheckoutId = currentCheckoutId;
        // Counts belong to the checkout they were counted in, and a path can be the same
        // in two checkouts, so the previous checkout's numbers never outlive the selection.
        state.fileCounts = {};
      }
      scheduleRefresh();
    },
    { immediate: true },
  );

  onScopeDispose(() => {
    if (--consumers > 0) return;
    unlisten?.();
    unlisten = undefined;
    // The last reader is gone, so nothing may answer with numbers nobody is showing.
    activeCheckoutId = null;
    state.checkoutTotals = {};
    state.fileCounts = {};
  });

  return state;
}

async function startListening() {
  try {
    unlisten = await listen<string>("git-status-changed", () => scheduleRefresh());
  } catch {
    // Without the event the counts refresh on selection and on workspace changes only, which
    // is stale but never wrong.
  }
}

/** A burst of changes is one round trip, the way the file list debounces its own refresh. */
function scheduleRefresh() {
  if (refreshQueued) return;
  refreshQueued = true;
  queueMicrotask(() => {
    refreshQueued = false;
    void refresh();
  });
}

async function refresh() {
  const request = ++generation;
  const checkoutId = activeCheckoutId;
  // One failure must not take the other half down: a checkout with no readable counts leaves
  // the sidebar totals it already had, and the Changes tab loses only its numbers.
  const [totals, files] = await Promise.all([
    getGitCheckoutDiffStats().catch(() => null),
    checkoutId ? getGitDiffStats(checkoutId).catch(() => null) : Promise.resolve(null),
  ]);
  if (request !== generation) return;
  if (totals) {
    for (const id of Object.keys(state.checkoutTotals)) delete state.checkoutTotals[id];
    Object.assign(state.checkoutTotals, totals);
  }
  if (files) {
    const counts: Record<string, DiffStatsFile> = {};
    for (const file of files) counts[file.path] = { additions: file.additions, deletions: file.deletions };
    state.fileCounts = counts;
  }
}
