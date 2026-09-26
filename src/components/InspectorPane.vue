<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { computed, nextTick, ref, watch } from "vue";
import { isIpcError } from "../domain/ipc";
import type { FileEntry, FileSearchResult } from "../domain/files";
import type { Checkout, Repo } from "../domain/workspace";
import type { CheckoutUiState } from "../domain/ui-state";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { AgentSession } from "../domain/agent";
import type { ActiveReviewNotes } from "../presentation/review-notes";
import { listCheckoutFiles, searchCheckoutFiles } from "../lib/ipc";
import ChangesPane from "./ChangesPane.vue";

const props = defineProps<{
  checkout: Checkout | null;
  repo?: Repo | null;
  gitSnapshot: ActiveGitSnapshot;
  review?: Pick<ActiveReviewNotes, "notes" | "rounds" | "markSent">;
  agentSessions?: AgentSession[];
  agentTargetId?: string | null;
  commandRequest?: { action: "open-file" | "open-changes"; token: number } | null;
  savedState?: CheckoutUiState | null;
}>();
/** Forwards the send and its mode: the choice the user made belongs to the caller. */
function forwardSendReview(ids: string[], queue: boolean) {
  emit("sendReview", ids, queue);
}

const emit = defineEmits<{
  openFile: [value: { checkoutId: string; path: string }];
  openChange: [value: { checkoutId: string; path: string }];
  sendReview: [ids: string[], queue: boolean];
  selectAgentTarget: [sessionId: string];
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

type DirectoryState = "loading" | "error" | "empty" | "truncated";
interface VisibleEntry {
  entry?: FileEntry;
  depth: number;
  message?: string;
}

const directories = ref<Record<string, FileEntry[]>>({});
const directoryStates = ref<Record<string, DirectoryState>>({});
const expanded = ref<string[]>([]);
const rootState = ref<"idle" | "loading" | "ready" | "error" | "missing">("idle");
const rootError = ref("");
const selectedPaths = ref<Record<string, string | null>>({});
const selectedChangedPaths = ref<Record<string, string | null>>({});
const selectedPath = computed(() => (props.checkout ? (selectedPaths.value[props.checkout.id] ?? null) : null));
const selectedChangedPath = computed(() =>
  props.checkout ? (selectedChangedPaths.value[props.checkout.id] ?? null) : null,
);
const activeTab = ref<"files" | "changes">("files");
const searchQuery = ref("");
const searchInput = ref<HTMLInputElement | null>(null);
const searchEntries = ref<FileEntry[]>([]);
const searchTruncated = ref(false);
const searchState = ref<"idle" | "loading" | "ready" | "error">("idle");
const searchError = ref("");
const treeScrollTop = ref(0);
const changesScrollTop = ref(0);
const treeViewport = ref<HTMLElement | null>(null);
let searchIndexCheckoutId: string | null = null;
let searchIndexPromise: { checkoutId: string; promise: Promise<FileSearchResult> } | null = null;
let generation = 0;
let searchGeneration = 0;
let refreshMicrotaskQueued = false;
let refreshInFlight = false;
let pendingRefreshCheckoutId: string | null = null;

const TREE_ROW_HEIGHT = 28;
const TREE_WINDOW_SIZE = 80;
const TREE_OVERSCAN = 12;

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
    if (requestGeneration !== generation) return;
    const message = errorText(error);
    if (path === ".") {
      if (!hasCachedEntries) {
        rootState.value = isIpcError(error) && error.code === "folder_missing" ? "missing" : "error";
      }
      rootError.value = message;
    } else if (!hasCachedEntries) {
      directoryStates.value = { ...directoryStates.value, [path]: "error" };
      rootError.value = message;
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
    searchQuery.value = "";
    searchEntries.value = [];
    searchTruncated.value = false;
    searchIndexCheckoutId = null;
    searchIndexPromise = null;
    searchState.value = "idle";
    searchGeneration += 1;
    const saved = props.savedState;
    treeScrollTop.value = saved?.filesScrollTop ?? 0;
    changesScrollTop.value = saved?.changesScrollTop ?? 0;
    expanded.value = saved?.expandedDirectories ?? [];
    activeTab.value = saved?.inspectorTab === "changes" && props.repo?.kind === "git" ? "changes" : "files";
    if (checkoutId && saved?.selectedFilePath)
      selectedPaths.value = { ...selectedPaths.value, [checkoutId]: saved.selectedFilePath };
    if (checkoutId && saved?.selectedChangePath)
      selectedChangedPaths.value = { ...selectedChangedPaths.value, [checkoutId]: saved.selectedChangePath };
    rootError.value = "";
    if (!checkoutId) {
      rootState.value = "idle";
      return;
    }
    if (isMissing) {
      rootState.value = "missing";
      rootError.value = "Checkout is no longer available.";
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

function onChangesScroll(top: number) {
  changesScrollTop.value = top;
}

function onInspectorTabKeydown(event: KeyboardEvent) {
  if (!(event.target instanceof HTMLElement) || event.target.getAttribute("role") !== "tab") return;
  const tabs = [
    "files",
    ...(props.repo?.kind === "git" && props.checkout && !props.checkout.isMissing ? ["changes"] : []),
  ];
  const current = event.target.id === "inspector-tab-changes" ? "changes" : "files";
  const index = tabs.indexOf(current);
  const next =
    event.key === "Home"
      ? tabs[0]
      : event.key === "End"
        ? tabs[tabs.length - 1]
        : event.key === "ArrowRight"
          ? tabs[(index + 1) % tabs.length]
          : event.key === "ArrowLeft"
            ? tabs[(index + tabs.length - 1) % tabs.length]
            : null;
  if (!next) return;
  event.preventDefault();
  activeTab.value = next as "files" | "changes";
  void nextTick(() => document.getElementById(`inspector-tab-${next}`)?.focus());
}

watch(
  () => props.commandRequest?.token,
  async () => {
    const request = props.commandRequest;
    if (!request) return;
    if (request.action === "open-changes" && props.repo?.kind === "git") activeTab.value = "changes";
    else if (request.action === "open-file") {
      activeTab.value = "files";
      await nextTick();
      searchInput.value?.focus();
    }
  },
);

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
  if (searchQuery.value.trim() && props.checkout?.id === checkoutId) {
    await refreshSearchIndex(checkoutId);
  }
}

watch(
  () => props.repo?.kind,
  (kind) => {
    if (kind !== "git") activeTab.value = "files";
  },
);

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

function selectChange(selection: { checkoutId: string; path: string }) {
  const { checkoutId, path } = selection;
  if (props.checkout?.id !== checkoutId) return;
  selectedChangedPaths.value = { ...selectedChangedPaths.value, [checkoutId]: path };
  emit("openChange", { checkoutId, path });
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
        result.push({ depth: depth + 1, message: rootError.value || "Could not load folder." });
      else if (state === "empty") result.push({ depth: depth + 1, message: "Empty folder." });
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

async function loadSearchIndex(checkoutId: string, force = false): Promise<FileSearchResult> {
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

async function refreshSearchIndex(checkoutId: string) {
  const searchRequest = ++searchGeneration;
  const hadCachedIndex = searchIndexCheckoutId === checkoutId;
  searchIndexCheckoutId = null;
  if (!hadCachedIndex) {
    searchEntries.value = [];
    searchTruncated.value = false;
    searchState.value = "loading";
  }
  try {
    await loadSearchIndex(checkoutId, true);
    if (searchRequest === searchGeneration) searchState.value = "ready";
  } catch (error) {
    if (searchRequest !== searchGeneration) return;
    if (hadCachedIndex) searchState.value = "ready";
    else {
      searchError.value = errorText(error);
      searchState.value = "error";
    }
  }
}

watch(
  () => [props.checkout?.id, props.checkout?.isMissing, searchQuery.value] as const,
  async ([checkoutId, isMissing, query]) => {
    const requestGeneration = ++searchGeneration;
    treeScrollTop.value = 0;
    searchError.value = "";
    if (!query.trim() || !checkoutId || isMissing) {
      searchState.value = "idle";
      return;
    }
    searchState.value = "loading";
    try {
      await loadSearchIndex(checkoutId);
      if (requestGeneration === searchGeneration) searchState.value = "ready";
    } catch (error) {
      if (requestGeneration === searchGeneration) {
        searchError.value = errorText(error);
        searchState.value = "error";
      }
    }
  },
);

const matchedSearchEntries = computed<VisibleEntry[]>(() => {
  const query = searchQuery.value.trim();
  if (!query) return [];
  return searchEntries.value
    .map((entry) => ({ entry, score: fuzzyScore(entry.path, query) }))
    .filter((match) => match.score >= 0)
    .sort((left, right) => left.score - right.score || left.entry.path.localeCompare(right.entry.path))
    .slice(0, 200)
    .map(({ entry }) => ({ entry, depth: 0 }));
});

const fileRows = computed<VisibleEntry[]>(() =>
  searchQuery.value.trim() ? matchedSearchEntries.value : visibleEntries.value,
);

const visibleTreeWindow = computed(() => {
  const maximumStart = Math.max(0, fileRows.value.length - TREE_WINDOW_SIZE);
  const start = Math.min(maximumStart, Math.max(0, Math.floor(treeScrollTop.value / TREE_ROW_HEIGHT) - TREE_OVERSCAN));
  const end = Math.min(fileRows.value.length, start + TREE_WINDOW_SIZE);
  return {
    rows: fileRows.value.slice(start, end),
    paddingTop: start * TREE_ROW_HEIGHT,
    paddingBottom: (fileRows.value.length - end) * TREE_ROW_HEIGHT,
  };
});

const gitDecorations = computed(() => {
  const decorations = new Map<string, string>();
  for (const file of props.gitSnapshot.status?.files ?? []) {
    decorations.set(file.path, file.status);
    const parents = file.path.split("/");
    parents.pop();
    for (let index = 1; index <= parents.length; index += 1) {
      const directory = parents.slice(0, index).join("/");
      if (!decorations.has(directory)) decorations.set(directory, "•");
    }
  }
  return decorations;
});

function decorationFor(path: string): string | undefined {
  return gitDecorations.value.get(path);
}

function onTreeScroll(event: Event) {
  treeScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
}
</script>

<template>
  <aside class="app-inspector flex h-full w-full min-w-0 flex-col border-l">
    <div
      role="tablist"
      aria-label="Inspector sections"
      class="inspector-tablist flex h-10 shrink-0 items-center gap-0.5 border-b px-2"
      @keydown="onInspectorTabKeydown"
    >
      <button
        v-if="checkout"
        id="inspector-tab-files"
        role="tab"
        type="button"
        :aria-selected="activeTab === 'files'"
        aria-controls="inspector-panel-files"
        :tabindex="activeTab === 'files' ? 0 : -1"
        class="inspector-tab rounded-sm px-2.5 py-1 text-[11px] font-medium tracking-wide"
        @click="activeTab = 'files'"
      >
        Files
      </button>
      <button
        v-if="repo?.kind === 'git' && checkout && !checkout.isMissing"
        id="inspector-tab-changes"
        role="tab"
        type="button"
        :aria-selected="activeTab === 'changes'"
        aria-controls="inspector-panel-changes"
        :tabindex="activeTab === 'changes' ? 0 : -1"
        class="inspector-tab rounded-sm px-2.5 py-1 text-[11px] font-medium tracking-wide"
        @click="activeTab = 'changes'"
      >
        Changes
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
      <div v-if="checkout && !checkout.isMissing" class="shrink-0 border-b p-1.5">
        <input
          ref="searchInput"
          v-model="searchQuery"
          type="search"
          aria-label="Search files"
          placeholder="Search files…"
          class="inspector-filter h-7 w-full rounded-sm px-2 text-xs outline-none placeholder:text-zinc-600"
        />
      </div>
      <section
        ref="treeViewport"
        class="min-h-0 flex-1 overflow-auto p-1"
        aria-label="Checkout files"
        @scroll="onTreeScroll"
      >
        <p v-if="rootState === 'idle'" class="pane-state text-sm">Open a checkout to browse files.</p>
        <p v-else-if="rootState === 'loading'" role="status" class="pane-state text-sm">Loading files…</p>
        <p v-else-if="rootState === 'missing'" role="status" class="pane-state text-sm text-amber-300">
          Checkout is missing.
        </p>
        <p v-else-if="rootState === 'error'" role="alert" class="pane-state text-sm text-red-300">{{ rootError }}</p>
        <p
          v-else-if="directories['.']?.length === 0 && directoryStates['.'] !== 'truncated'"
          role="status"
          class="pane-state text-sm"
        >
          This checkout is empty.
        </p>
        <template v-else>
          <p
            v-if="searchQuery.trim() && searchState === 'loading'"
            role="status"
            class="px-2 py-2 text-xs text-zinc-500"
          >
            Searching checkout files…
          </p>
          <p
            v-else-if="searchQuery.trim() && searchState === 'error'"
            role="alert"
            class="px-2 py-2 text-xs text-red-300"
          >
            {{ searchError }}
          </p>
          <template v-else-if="searchQuery.trim() && searchState === 'ready'">
            <p v-if="matchedSearchEntries.length === 0" role="status" class="px-2 py-2 text-xs text-zinc-500">
              No files match this search.
            </p>
            <p v-else-if="matchedSearchEntries.length === 200" class="px-2 pb-1 text-[10px] text-zinc-600">
              Showing the best 200 matches; refine the search for more.
            </p>
          </template>
          <p v-if="searchQuery.trim() && searchTruncated" class="px-2 pb-1 text-[10px] text-amber-300">
            Search is limited to the first 50,000 files in this checkout.
          </p>
          <p v-if="directoryStates['.'] === 'truncated'" class="px-2 py-1 text-xs text-zinc-500">
            Some entries omitted (folder is large).
          </p>
          <p v-if="searchQuery.trim() && searchState !== 'ready'" class="px-2 py-1 text-xs text-zinc-500">
            Search filters files across the checkout.
          </p>
          <div
            v-else
            :style="{
              paddingTop: `${visibleTreeWindow.paddingTop}px`,
              paddingBottom: `${visibleTreeWindow.paddingBottom}px`,
            }"
          >
            <div
              v-for="(item, index) in visibleTreeWindow.rows"
              :key="item.entry?.path ?? `${item.depth}-${index}-${item.message}`"
              :style="{ paddingLeft: `${4 + item.depth * 12}px` }"
              class="flex h-7 items-center overflow-hidden"
            >
              <span v-if="!item.entry" class="truncate py-1 text-xs text-zinc-500">{{ item.message }}</span>
              <button
                v-else-if="item.entry.kind === 'directory'"
                type="button"
                class="inspector-tree-row flex h-7 w-full min-w-0 items-center truncate rounded-sm px-1 text-left text-xs text-zinc-300"
                :aria-expanded="expanded.includes(item.entry.path)"
                @click="toggleDirectory(item.entry)"
              >
                <span class="mr-2 shrink-0 text-zinc-500">
                  {{ expanded.includes(item.entry.path) ? "▾" : "▸" }}
                </span>
                <span class="truncate">{{ item.entry.name }}</span>
                <span v-if="decorationFor(item.entry.path)" class="ml-auto pl-2 text-[10px] text-amber-300">
                  {{ decorationFor(item.entry.path) }}
                </span>
              </button>
              <button
                v-else
                type="button"
                class="inspector-tree-row flex h-7 w-full min-w-0 items-center truncate rounded-sm px-1 text-left text-xs"
                :class="selectedPath === item.entry.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
                @click="selectFile(item.entry)"
              >
                <span class="mr-2 shrink-0 text-zinc-600">
                  {{ item.entry.kind === "symlink" ? "↗" : "·" }}
                </span>
                <span class="truncate">{{ item.entry.name }}</span>
                <span
                  v-if="decorationFor(item.entry.path)"
                  class="ml-auto pl-2 text-[10px] font-semibold"
                  :class="decorationFor(item.entry.path) === '??' ? 'text-green-400' : 'text-amber-300'"
                >
                  {{ decorationFor(item.entry.path) }}
                </span>
              </button>
            </div>
          </div>
        </template>
      </section>
    </div>
    <div
      v-if="checkout && !checkout.isMissing && repo?.kind === 'git'"
      v-show="activeTab === 'changes'"
      id="inspector-panel-changes"
      role="tabpanel"
      aria-labelledby="inspector-tab-changes"
      tabindex="0"
      class="flex min-h-0 flex-1 flex-col"
    >
      <ChangesPane
        :key="checkout.id"
        :checkout="checkout"
        :git-snapshot="gitSnapshot"
        :review="review"
        :agent-sessions="agentSessions"
        :agent-target-id="agentTargetId"
        :selected-path="selectedChangedPath"
        :scroll-top="changesScrollTop"
        @open-change="selectChange"
        @scroll-position-changed="onChangesScroll"
        @send-review="forwardSendReview"
        @select-agent-target="$emit('selectAgentTarget', $event)"
      />
    </div>
  </aside>
</template>
