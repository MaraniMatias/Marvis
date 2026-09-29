<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { ChevronRight as ChevronRightIcon, SquareArrowOutUpRight as SquareArrowOutUpRightIcon } from "@lucide/vue";
import { isIpcError } from "../domain/ipc";
import type { FileEntry } from "../domain/files";
import type { Checkout, Repo } from "../domain/workspace";
import type { CheckoutUiState } from "../domain/ui-state";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useDiffStats } from "../presentation/diff-stats";
import { listCheckoutFiles } from "../lib/ipc";
import FileIcon from "./FileIcon.vue";

const props = defineProps<{
  checkout: Checkout | null;
  repo?: Repo | null;
  gitSnapshot: ActiveGitSnapshot;
  savedState?: CheckoutUiState | null;
}>();

const emit = defineEmits<{
  openFile: [value: { checkoutId: string; path: string }];
  openChange: [value: { checkoutId: string; path: string }];
  /** Every changed file in one diff, which is the one view with no path. */
  openAllChanges: [value: { checkoutId: string }];
  updateUiState: [
    value: Pick<
      CheckoutUiState,
      | "inspectorTab"
      | "selectedFilePath"
      | "selectedChangePath"
      | "expandedDirectories"
      | "filesScrollTop"
      | "changesScrollTop"
    >,
  ];
}>();

type InspectorTab = "files" | "changes";
type DirectoryState = "loading" | "error" | "empty" | "truncated";
/** The file tree rows carry the same optional counts as the change rows, but nothing fills
 *  them in yet: the tree lists the whole checkout, not its diff, and the counts belong to the
 *  diff view that the file rows are about to become. */
interface DiffStats {
  additions?: number;
  deletions?: number;
}
interface VisibleEntry extends DiffStats {
  entry?: FileEntry;
  depth: number;
  message?: string;
}
/** A change row is either a group header or one of its files. Both are one row tall, so the
 *  grouped list virtualizes with the same math as the tree. */
type ChangeFile = { kind: "file"; key: string; name: string; status: string; oldPath?: string } & DiffStats;
type ChangeRow = ChangeFile | { kind: "group"; key: string; dir: string };

// The counts come from the same store the sidebar reads and from the same base ref the file
// list below is built from, so a row's number always describes the change it sits next to.
const diffStats = useDiffStats(
  computed(() => (props.repo ? [props.repo] : [])),
  computed(() => (props.repo?.kind === "git" ? (props.checkout?.id ?? null) : null)),
);

const directories = ref<Record<string, FileEntry[]>>({});
const directoryStates = ref<Record<string, DirectoryState>>({});
const expanded = ref<string[]>([]);
const rootState = ref<"idle" | "loading" | "ready" | "error" | "missing">("idle");
/** The last reason the tree's root could not be listed. It has no row of its own, so the
 *  panel itself is where the reason stays: a toast expires and leaves the tree unexplained. */
const rootError = ref("");
/** The last reason a folder could not be listed, which its own row in the tree shows (E.2). */
const folderError = ref("");
const selectedPaths = ref<Record<string, string | null>>({});
const selectedChangedPaths = ref<Record<string, string | null>>({});
const selectedPath = computed(() => (props.checkout ? (selectedPaths.value[props.checkout.id] ?? null) : null));
const selectedChangedPath = computed(() =>
  props.checkout ? (selectedChangedPaths.value[props.checkout.id] ?? null) : null,
);
const activeTab = ref<InspectorTab>("files");
const treeScrollTop = ref(0);
const changesScrollTop = ref(0);
const treeViewport = ref<HTMLElement | null>(null);
const changesViewport = ref<HTMLElement | null>(null);
let generation = 0;
let refreshMicrotaskQueued = false;
let refreshInFlight = false;
let pendingRefreshCheckoutId: string | null = null;

/**
 * Every row of both lists is this tall, and `--tree-row-height` below is set from it, so the
 * `.file-row` box is `3px + (22 - 6px) line box + 3px` = 22px exactly. Change one without the
 * other and the virtual window drifts from the rendered rows. 80 rows x 28px was 2240px of
 * tree; the same surface at 22px is ~64 rows, with ~10 of them as overscan.
 */
const TREE_ROW_HEIGHT = 22;
const TREE_WINDOW_SIZE = 64;
const TREE_OVERSCAN = 10;

/** The status vocabulary the row decorations know (E.5): no `??`, no per-folder bubble. */
const ROW_STATUSES = new Set(["M", "A", "D", "U"]);

const showChanges = computed(() => props.repo?.kind === "git" && !!props.checkout && !props.checkout.isMissing);
const tabs = computed((): { id: InspectorTab; label: string }[] => {
  if (!props.checkout) return [];
  const result: { id: InspectorTab; label: string }[] = [{ id: "files", label: "Files" }];
  if (showChanges.value) result.push({ id: "changes", label: "Changes" });
  return result;
});

function errorText(error: unknown): string {
  if (isIpcError(error)) {
    switch (error.code) {
      case "folder_missing":
        return "File or folder no longer exists.";
      case "permission_denied":
        return "Permission denied while reading this folder or file.";
      case "file_too_large":
        return "This file is larger than the preview size limit.";
      case "binary_file":
        return "This file is binary or is not valid UTF-8.";
      case "path_outside_checkout":
        return "This path resolves outside the active checkout and cannot be opened.";
      case "invalid_path":
        return "This path is not a valid file or folder in the checkout.";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : String(error);
}

async function loadDirectory(checkoutId: string, path: string, requestGeneration: number) {
  const hasCachedEntries = Object.hasOwn(directories.value, path);
  if (!hasCachedEntries) {
    directoryStates.value = { ...directoryStates.value, [path]: "loading" };
    if (path === ".") rootState.value = "loading";
  } else {
    const currentState = directoryStates.value[path];
    if (currentState === "loading" || currentState === "error") {
      const nextStates = { ...directoryStates.value };
      delete nextStates[path];
      if (directories.value[path].length === 0) nextStates[path] = "empty";
      directoryStates.value = nextStates;
    }
    if (path === "." && rootState.value === "error") rootState.value = "ready";
  }
  try {
    const result = await listCheckoutFiles(checkoutId, path);
    if (requestGeneration !== generation) return;
    directories.value = { ...directories.value, [path]: result.entries };
    const nextStates = { ...directoryStates.value };
    delete nextStates[path];
    if (result.truncated) nextStates[path] = "truncated";
    else if (result.entries.length === 0) nextStates[path] = "empty";
    directoryStates.value = nextStates;
    if (path === ".") rootState.value = "ready";
  } catch (error) {
    if (requestGeneration !== generation || hasCachedEntries) return;
    const message = errorText(error);
    if (path === ".") {
      // The root has no row of its own to hold the reason, so the panel says it: a toast
      // expires and would leave the tree empty with nothing to explain it.
      if (isIpcError(error) && error.code === "folder_missing") {
        rootState.value = "missing";
        rootError.value = "";
      } else {
        rootState.value = "error";
        rootError.value = message;
      }
    } else {
      // A folder keeps it on its own row, where it lasts as long as the row does.
      directoryStates.value = { ...directoryStates.value, [path]: "error" };
      folderError.value = message;
    }
  }
}

watch(
  () => [props.checkout?.id, props.checkout?.isMissing] as const,
  async ([checkoutId, isMissing]) => {
    const requestGeneration = ++generation;
    directories.value = {};
    directoryStates.value = {};
    expanded.value = [];
    const saved = props.savedState;
    treeScrollTop.value = saved?.filesScrollTop ?? 0;
    changesScrollTop.value = saved?.changesScrollTop ?? 0;
    expanded.value = saved?.expandedDirectories ?? [];
    activeTab.value = saved?.inspectorTab === "changes" && showChanges.value ? "changes" : "files";
    if (checkoutId && saved?.selectedFilePath)
      selectedPaths.value = { ...selectedPaths.value, [checkoutId]: saved.selectedFilePath };
    if (checkoutId && saved?.selectedChangePath)
      selectedChangedPaths.value = { ...selectedChangedPaths.value, [checkoutId]: saved.selectedChangePath };
    folderError.value = "";
    rootError.value = "";
    if (!checkoutId) {
      rootState.value = "idle";
      return;
    }
    if (isMissing) {
      rootState.value = "missing";
      return;
    }
    rootState.value = "loading";
    await loadDirectory(checkoutId, ".", requestGeneration);
    const restoredDirectories = [...expanded.value];
    for (let index = 0; index < restoredDirectories.length; index += 16) {
      if (requestGeneration !== generation) return;
      await Promise.all(
        restoredDirectories.slice(index, index + 16).map((path) => loadDirectory(checkoutId, path, requestGeneration)),
      );
    }
    await nextTick();
    if (requestGeneration === generation && treeViewport.value) treeViewport.value.scrollTop = treeScrollTop.value;
  },
  { immediate: true },
);

watch([activeTab, selectedPath, selectedChangedPath, expanded, treeScrollTop, changesScrollTop], () => {
  if (!props.checkout) return;
  emit("updateUiState", {
    inspectorTab: activeTab.value,
    selectedFilePath: selectedPath.value,
    selectedChangePath: selectedChangedPath.value,
    expandedDirectories: expanded.value,
    filesScrollTop: Math.round(treeScrollTop.value),
    changesScrollTop: Math.round(changesScrollTop.value),
  });
});

function onInspectorTabKeydown(event: KeyboardEvent) {
  if (!(event.target instanceof HTMLElement) || event.target.getAttribute("role") !== "tab") return;
  const ids = tabs.value.map((tab) => tab.id);
  const current: InspectorTab = event.target.id === "inspector-tab-changes" ? "changes" : "files";
  const index = ids.indexOf(current);
  const next =
    event.key === "Home"
      ? ids[0]
      : event.key === "End"
        ? ids[ids.length - 1]
        : event.key === "ArrowRight"
          ? ids[(index + 1) % ids.length]
          : event.key === "ArrowLeft"
            ? ids[(index + ids.length - 1) % ids.length]
            : null;
  if (!next) return;
  event.preventDefault();
  activeTab.value = next;
  void nextTick(() => document.getElementById(`inspector-tab-${next}`)?.focus());
}

watch(
  () => [props.gitSnapshot.statusEventRevision, props.gitSnapshot.statusEventCheckoutId] as const,
  ([, eventCheckoutId]) => {
    const checkoutId = props.checkout?.id;
    if (
      checkoutId &&
      eventCheckoutId === checkoutId &&
      checkoutId === props.gitSnapshot.checkoutId &&
      !props.checkout?.isMissing &&
      props.repo?.kind === "git" &&
      activeTab.value !== "changes"
    ) {
      requestRefresh(checkoutId);
    }
  },
);

watch(activeTab, (tab) => {
  const checkoutId = props.checkout?.id;
  if (tab === "files" && checkoutId && props.repo?.kind === "git") {
    requestRefresh(checkoutId);
  }
});

watch(
  () => props.repo?.kind,
  (kind) => {
    if (kind !== "git") activeTab.value = "files";
  },
);

function requestRefresh(checkoutId: string) {
  pendingRefreshCheckoutId = checkoutId;
  if (refreshInFlight || refreshMicrotaskQueued) return;
  refreshMicrotaskQueued = true;
  queueMicrotask(() => {
    refreshMicrotaskQueued = false;
    const requestedCheckoutId = pendingRefreshCheckoutId;
    pendingRefreshCheckoutId = null;
    if (!requestedCheckoutId) return;
    refreshInFlight = true;
    void refreshAfterGitChange(requestedCheckoutId).finally(() => {
      refreshInFlight = false;
      if (pendingRefreshCheckoutId) requestRefresh(pendingRefreshCheckoutId);
    });
  });
}

async function refreshAfterGitChange(checkoutId: string) {
  const requestGeneration = generation;
  const paths = [
    ".",
    ...Object.keys(directories.value).filter((path) => path !== "." && expanded.value.includes(path)),
  ];
  for (const path of paths) {
    if (requestGeneration !== generation || props.checkout?.id !== checkoutId) return;
    await loadDirectory(checkoutId, path, requestGeneration);
  }
}

async function toggleDirectory(entry: FileEntry) {
  if (expanded.value.includes(entry.path)) {
    expanded.value = expanded.value.filter((path) => path !== entry.path);
    return;
  }
  expanded.value = [...expanded.value, entry.path];
  const checkoutId = props.checkout?.id;
  if (checkoutId && !Object.hasOwn(directories.value, entry.path)) {
    await loadDirectory(checkoutId, entry.path, generation);
  }
}

function selectFile(entry: FileEntry) {
  const checkoutId = props.checkout?.id;
  if (!checkoutId) return;
  selectedPaths.value = { ...selectedPaths.value, [checkoutId]: entry.path };
  emit("openFile", { checkoutId, path: entry.path });
}

function selectChange(path: string) {
  const checkoutId = props.checkout?.id;
  if (!checkoutId) return;
  selectedChangedPaths.value = { ...selectedChangedPaths.value, [checkoutId]: path };
  emit("openChange", { checkoutId, path });
}

function openAllChanges() {
  const checkoutId = props.checkout?.id;
  if (checkoutId) emit("openAllChanges", { checkoutId });
}

const visibleEntries = computed<VisibleEntry[]>(() => {
  const result: VisibleEntry[] = [];
  function append(path: string, depth: number) {
    for (const entry of directories.value[path] ?? []) {
      result.push({ entry, depth });
      if (entry.kind !== "directory" || !expanded.value.includes(entry.path)) continue;
      const state = directoryStates.value[entry.path];
      if (state === "loading") result.push({ depth: depth + 1, message: "Loading folder…" });
      else if (state === "error")
        result.push({ depth: depth + 1, message: folderError.value || "Could not load folder." });
      // An empty folder says nothing: it stays, and a line of prose under every one of them
      // is noise on a real tree.
      else {
        if (state === "truncated")
          result.push({ depth: depth + 1, message: "Some entries omitted (folder is large)." });
        append(entry.path, depth + 1);
      }
    }
  }
  append(".", 0);
  return result;
});

const changedFiles = computed(() => props.gitSnapshot.status?.files ?? []);
const changedFileCount = computed(() => changedFiles.value.length);
/** Grouped by parent directory, then flattened: a group header is a row of the same height, so
 *  the whole list windows with `virtualWindow`. Files at the root are labelled "/". */
const changeRows = computed<ChangeRow[]>(() => {
  const counts = diffStats.fileCounts;
  const groups = new Map<string, ChangeFile[]>();
  for (const file of changedFiles.value) {
    const slash = file.path.lastIndexOf("/");
    const dir = slash < 0 ? "" : file.path.slice(0, slash);
    const row: ChangeFile = {
      kind: "file",
      key: file.path,
      name: slash < 0 ? file.path : file.path.slice(slash + 1),
      status: file.status,
      oldPath: file.oldPath,
      // A file Git cannot count keeps both absent, so the row shows no number at all.
      additions: counts[file.path]?.additions,
      deletions: counts[file.path]?.deletions,
    };
    const bucket = groups.get(dir);
    if (bucket) bucket.push(row);
    else groups.set(dir, [row]);
  }
  const rows: ChangeRow[] = [];
  for (const [dir, files] of groups) rows.push({ kind: "group", key: `group:${dir}`, dir }, ...files);
  return rows;
});

const gitStatuses = computed(() => {
  const statuses = new Map<string, string>();
  for (const file of props.gitSnapshot.status?.files ?? []) {
    if (ROW_STATUSES.has(file.status)) statuses.set(file.path, file.status);
  }
  return statuses;
});

/**
 * How loud a row is drawn. A dotfile steps down once and a file the checkout ignores steps down
 * again, so a build directory reads as present but uninteresting. The flag comes from the
 * backend; the dot comes from the name, since nothing in Git treats it specially.
 */
function prominenceOf(entry: FileEntry): "normal" | "hidden" | "ignored" {
  if (entry.ignored) return "ignored";
  return entry.name.startsWith(".") ? "hidden" : "normal";
}

function rowStatus(path: string): string | undefined {
  return gitStatuses.value.get(path);
}

function rowIndent(depth: number): string {
  return `${6 + depth * 14}px`;
}

/** Both lists are flat and every row is TREE_ROW_HEIGHT tall, so one window serves both. */
function virtualWindow<T>(rows: T[], scrollTop: number) {
  const maximumStart = Math.max(0, rows.length - TREE_WINDOW_SIZE);
  const start = Math.min(maximumStart, Math.max(0, Math.floor(scrollTop / TREE_ROW_HEIGHT) - TREE_OVERSCAN));
  const end = Math.min(rows.length, start + TREE_WINDOW_SIZE);
  return {
    rows: rows.slice(start, end),
    paddingTop: start * TREE_ROW_HEIGHT,
    paddingBottom: (rows.length - end) * TREE_ROW_HEIGHT,
  };
}

const treeWindow = computed(() => virtualWindow(visibleEntries.value, treeScrollTop.value));
const changeWindow = computed(() => virtualWindow(changeRows.value, changesScrollTop.value));

function onTreeScroll(event: Event) {
  treeScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
}

function onChangesScroll(event: Event) {
  changesScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
}

watch(
  () => [props.checkout?.id, props.gitSnapshot.statusState] as const,
  async ([, state]) => {
    if (state !== "ready") return;
    const savedTop = changesScrollTop.value;
    await nextTick();
    if (changesViewport.value) changesViewport.value.scrollTop = savedTop;
  },
  { immediate: true, flush: "post" },
);

/* E.1: the file search is out of this panel, and the block below is the whole search path,
   commented out rather than deleted because the decision is reversible: `searchCheckoutFiles`
   (src/lib/ipc.ts) and this scorer are untouched, and the input that fed them is the only
   thing missing. Restoring it means uncommenting this block plus `searchCheckoutFiles` and
   `FileSearchResult` in the imports, putting the input back in the files panel, and letting
   the tree window read the matches instead of `visibleEntries`.

function fuzzyScore(path: string, query: string): number {
  const candidate = path.toLocaleLowerCase();
  const normalized = query.toLocaleLowerCase().trim();
  let cursor = 0;
  let score = 0;
  let previous = -2;
  for (const character of normalized) {
    const index = candidate.indexOf(character, cursor);
    if (index < 0) return -1;
    score += index - cursor + (index === previous + 1 ? -2 : index);
    previous = index;
    cursor = index + 1;
  }
  const basename = candidate.slice(candidate.lastIndexOf("/") + 1);
  if (basename.startsWith(normalized)) score -= 100;
  return score;
}

async function loadSearchIndex(checkoutId: string, force = false) {
  if (!force && searchIndexCheckoutId === checkoutId) {
    return { entries: searchEntries.value, truncated: searchTruncated.value };
  }
  if (searchIndexPromise?.checkoutId === checkoutId) return searchIndexPromise.promise;
  const promise = searchCheckoutFiles(checkoutId);
  searchIndexPromise = { checkoutId, promise };
  try {
    const result = await promise;
    if (props.checkout?.id === checkoutId && searchIndexPromise?.promise === promise) {
      searchIndexCheckoutId = checkoutId;
      searchEntries.value = result.entries;
      searchTruncated.value = result.truncated;
    }
    return result;
  } finally {
    if (searchIndexPromise?.promise === promise) searchIndexPromise = null;
  }
}

const matchedSearchEntries = searchEntries.value
  .map((entry) => ({ entry, score: fuzzyScore(entry.path, searchQuery.value.trim()) }))
  .filter((match) => match.score >= 0)
  .sort((left, right) => left.score - right.score || left.entry.path.localeCompare(right.entry.path))
  .slice(0, 200)
  .map(({ entry }) => ({ entry, depth: 0 }));
*/
</script>

<template>
  <aside
    class="app-inspector flex h-full w-full min-w-0 flex-col border-l"
    :style="{ '--tree-row-height': `${TREE_ROW_HEIGHT}px` }"
  >
    <div class="details-tabs" role="tablist" aria-label="Inspector sections" @keydown="onInspectorTabKeydown">
      <button
        v-for="tab in tabs"
        :id="`inspector-tab-${tab.id}`"
        :key="tab.id"
        type="button"
        role="tab"
        class="details-tab"
        :aria-selected="activeTab === tab.id"
        :aria-controls="`inspector-panel-${tab.id}`"
        :tabindex="activeTab === tab.id ? 0 : -1"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
        <span v-if="tab.id === 'changes' && changedFileCount > 0" class="details-tab-count">
          {{ changedFileCount }}
        </span>
      </button>
    </div>

    <div
      v-show="activeTab === 'files'"
      :id="checkout ? 'inspector-panel-files' : undefined"
      :role="checkout ? 'tabpanel' : undefined"
      :aria-labelledby="checkout ? 'inspector-tab-files' : undefined"
      :tabindex="checkout ? 0 : undefined"
      class="flex min-h-0 flex-1 flex-col"
    >
      <div ref="treeViewport" class="details-scroll" aria-label="Checkout files" @scroll="onTreeScroll">
        <p v-if="rootState === 'idle'" class="pane-state">Open a checkout to browse files.</p>
        <p v-else-if="rootState === 'loading'" role="status" class="pane-state">Loading files…</p>
        <!-- The root's own failure is the panel's whole content, so it is drawn here instead of
             expiring in a toast over an empty tree. -->
        <p v-else-if="rootState === 'error'" role="alert" class="pane-state">
          {{ rootError || "Could not list this checkout." }}
        </p>
        <p v-else-if="rootState === 'missing'" role="status" class="pane-state">Checkout is missing.</p>
        <p
          v-else-if="directories['.']?.length === 0 && directoryStates['.'] !== 'truncated'"
          role="status"
          class="pane-state"
        >
          This checkout is empty.
        </p>
        <template v-else>
          <p v-if="directoryStates['.'] === 'truncated'" class="file-row file-note" :style="{ paddingLeft: '6px' }">
            Some entries omitted (folder is large).
          </p>
          <div
            :style="{
              paddingTop: `${treeWindow.paddingTop}px`,
              paddingBottom: `${treeWindow.paddingBottom}px`,
            }"
          >
            <template
              v-for="(item, index) in treeWindow.rows"
              :key="item.entry?.path ?? `${item.depth}-${index}-${item.message}`"
            >
              <p
                v-if="!item.entry"
                class="file-row file-note"
                :style="{ paddingLeft: rowIndent(item.depth) }"
                :title="item.message"
              >
                {{ item.message }}
              </p>
              <button
                v-else-if="item.entry.kind === 'directory'"
                type="button"
                class="file-row file-folder"
                :style="{ paddingLeft: rowIndent(item.depth) }"
                :aria-expanded="expanded.includes(item.entry.path)"
                :title="item.entry.path"
                @click="toggleDirectory(item.entry)"
              >
                <ChevronRightIcon
                  class="icon-xxs chevron"
                  :class="{ expanded: expanded.includes(item.entry.path) }"
                  aria-hidden="true"
                />
                <FileIcon
                  class="file-icon"
                  :name="item.entry.name"
                  kind="directory"
                  :prominence="prominenceOf(item.entry)"
                />
                <span class="file-name" :class="`file-name-${prominenceOf(item.entry)}`">
                  {{ item.entry.name }}
                </span>
              </button>
              <button
                v-else
                type="button"
                class="file-row"
                :class="{ 'is-selected': selectedPath === item.entry.path }"
                :style="{ paddingLeft: rowIndent(item.depth) }"
                :aria-current="selectedPath === item.entry.path ? 'true' : undefined"
                :title="item.entry.path"
                @click="selectFile(item.entry)"
              >
                <span class="chevron-spacer" aria-hidden="true" />
                <FileIcon
                  class="file-icon"
                  :name="item.entry.name"
                  kind="file"
                  :prominence="prominenceOf(item.entry)"
                />
                <span class="file-name" :class="`file-name-${prominenceOf(item.entry)}`">
                  {{ item.entry.name }}
                </span>
                <span v-if="rowStatus(item.entry.path)" class="file-status" :data-status="rowStatus(item.entry.path)">
                  {{ rowStatus(item.entry.path) }}
                </span>
                <span v-if="item.additions" class="diff-add">+{{ item.additions }}</span>
                <span v-if="item.deletions" class="diff-del">-{{ item.deletions }}</span>
              </button>
            </template>
          </div>
        </template>
      </div>
    </div>

    <div v-if="showChanges" v-show="activeTab === 'changes'" class="flex min-h-0 flex-1 flex-col">
      <div class="details-all-changes">
        <button type="button" class="file-row new-item" @click="openAllChanges">
          <SquareArrowOutUpRightIcon class="icon-xs file-icon" aria-hidden="true" />
          <span>All changes</span>
        </button>
      </div>
      <div
        id="inspector-panel-changes"
        role="tabpanel"
        aria-labelledby="inspector-tab-changes"
        tabindex="0"
        class="flex min-h-0 flex-1 flex-col"
      >
        <div ref="changesViewport" class="details-scroll" aria-label="Changed files" @scroll="onChangesScroll">
          <p v-if="gitSnapshot.statusState === 'loading'" role="status" class="pane-state">Loading Git status…</p>
          <p v-else-if="gitSnapshot.statusState === 'error'" role="alert" class="pane-state">
            {{ gitSnapshot.changesStatusError || gitSnapshot.statusError }}
          </p>
          <p v-else-if="changedFileCount === 0" role="status" class="pane-state">No changed files.</p>
          <template v-else>
            <p
              v-if="gitSnapshot.changesWatchError"
              class="file-row file-note"
              :style="{ paddingLeft: '6px' }"
              :title="`Live updates unavailable: ${gitSnapshot.changesWatchError}`"
            >
              Live updates unavailable: {{ gitSnapshot.changesWatchError }}
            </p>
            <div
              :style="{
                paddingTop: `${changeWindow.paddingTop}px`,
                paddingBottom: `${changeWindow.paddingBottom}px`,
              }"
            >
              <template v-for="row in changeWindow.rows" :key="row.key">
                <div v-if="row.kind === 'group'" class="details-group-header" :title="row.dir || '/'">
                  <span class="file-name">{{ row.dir || "/" }}</span>
                </div>
                <button
                  v-else
                  type="button"
                  class="file-row change-row"
                  :class="{ 'is-selected': selectedChangedPath === row.key }"
                  :aria-current="selectedChangedPath === row.key ? 'true' : undefined"
                  :title="row.oldPath ? `${row.oldPath} → ${row.key}` : row.key"
                  @click="selectChange(row.key)"
                >
                  <span class="file-status" :data-status="row.status">{{ row.status }}</span>
                  <span class="file-name">{{ row.name }}</span>
                  <span v-if="row.additions" class="diff-add">+{{ row.additions }}</span>
                  <span v-if="row.deletions" class="diff-del">-{{ row.deletions }}</span>
                </button>
              </template>
            </div>
          </template>
        </div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
/* The tab strip stays put */
.details-tabs {
  display: flex;
  align-items: stretch;
  flex-shrink: 0;
  border-bottom: 1px solid var(--marvis-border);
}

.details-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: -1px;
  padding: 7px 6px;
  border: none;
  border-bottom: 1px solid transparent;
  background: transparent;
  color: var(--marvis-text-faint);
  font-family: inherit;
  font-size: 11px;
  letter-spacing: 0.02em;
  text-align: left;
  cursor: pointer;
}

/* A tab is a segment of the strip, not a standalone control, so it keeps the mockup's
   transparent rest and accent underline, and takes the hover and pressed states on top. A
   resting surface here would break the strip into separate blocks. */
.details-tab:hover {
  color: var(--marvis-text-secondary);
  background: var(--marvis-bg-2);
}

.details-tab:active {
  background: var(--marvis-border);
}

.details-tab[aria-selected="true"] {
  color: var(--marvis-text);
  border-bottom-color: var(--marvis-accent);
}

.details-tab-count {
  color: var(--marvis-text-faint);
  font-size: 10px;
}

/* The tab strip stays put, each list scrolls on its own. The right padding is the scrollbar's:
   macOS draws its own overlay scrollbar on top of the content, so a row whose status letter or
   +/- counts end 6px from the edge is read through it while it is showing. */
.details-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 8px 4px 0;
}

/*
 * Every row in both lists is exactly TREE_ROW_HEIGHT tall, which the component publishes as
 * `--tree-row-height`: 3px of padding + a (height - 6px) line box + 3px of padding. The
 * virtual window multiplies and divides by the same number, so this box and TREE_ROW_HEIGHT
 * have to move together.
 */
.file-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  height: var(--tree-row-height);
  padding: 3px 6px;
  border: none;
  background: transparent;
  color: var(--marvis-text-secondary);
  font-family: inherit;
  font-size: 12px;
  line-height: calc(var(--tree-row-height) - 6px);
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  cursor: pointer;
}

.file-row:hover {
  background: var(--marvis-bg-2);
  color: var(--marvis-text);
}

/* What is open, or the change under review, is the one row with a surface (E.3) */
.file-row.is-selected {
  background: var(--marvis-bg-2);
  color: var(--marvis-text);
}

.file-folder {
  color: var(--marvis-text);
}

.file-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* The name steps with the icon, a dotfile and an ignored file both still readable rather than
   fading out. Quieting is opacity and not a mixed color, which is the whole reason: a color of its
   own would win over the one the row hands down, and these two numbers are the name half of the
   ramp that FileIcon's opacity rules make up the icon half. */
.file-name-hidden {
  opacity: 0.72;
}

.file-name-ignored {
  opacity: 0.48;
}

.file-icon,
.chevron {
  flex-shrink: 0;
  color: var(--marvis-text-faint);
}

.chevron {
  transition: transform 0.12s ease;
}

.chevron.expanded {
  transform: rotate(90deg);
}

.chevron-spacer {
  width: 10px;
  height: 10px;
  flex-shrink: 0;
}

/* Folder states and watch errors, one line tall like any other row */
.file-note {
  color: var(--marvis-text-faint);
  cursor: default;
}

.file-note:hover {
  background: transparent;
}

.file-status {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--marvis-text-secondary);
}

.file-status[data-status="M"] {
  color: var(--marvis-text);
}

.file-status[data-status="A"] {
  color: var(--marvis-green);
}

.file-status[data-status="D"] {
  color: var(--marvis-red);
}

.file-status[data-status="U"] {
  color: var(--marvis-text-faint);
}

.details-all-changes {
  flex-shrink: 0;
  padding: 2px 0;
}

.file-row.new-item {
  color: var(--marvis-text-dim);
}

/* Group header sits on the panel gutter, its rows one level in */
.details-group-header {
  display: flex;
  align-items: center;
  height: var(--tree-row-height);
  padding: 0 6px;
  color: var(--marvis-text-faint);
  font-size: 11px;
  line-height: var(--tree-row-height);
  letter-spacing: 0.02em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.change-row {
  padding-left: 20px;
}
</style>
