<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { computed, nextTick, ref, watch } from "vue";
import { isIpcError } from "../domain/ipc";
import type { FileEntry, FileSearchResult } from "../domain/files";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { listCheckoutFiles, searchCheckoutFiles } from "../lib/ipc";
import ChangesPane from "./ChangesPane.vue";

const props = defineProps<{
  checkout: Checkout | null;
  repo?: Repo | null;
  gitSnapshot: ActiveGitSnapshot;
  commandRequest?: { action: "open-file" | "open-changes"; token: number } | null;
}>();
const emit = defineEmits<{
  openFile: [value: { checkoutId: string; path: string }];
  openChange: [value: { checkoutId: string; path: string }];
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
let searchIndexCheckoutId: string | null = null;
let searchIndexPromise: { checkoutId: string; promise: Promise<FileSearchResult> } | null = null;
let generation = 0;
let searchGeneration = 0;

const TREE_ROW_HEIGHT = 32;
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
  directoryStates.value = { ...directoryStates.value, [path]: "loading" };
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
      rootState.value = isIpcError(error) && error.code === "folder_missing" ? "missing" : "error";
      rootError.value = message;
    } else {
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
    treeScrollTop.value = 0;
    activeTab.value = "files";
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
  },
  { immediate: true },
);

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
      void refreshAfterGitChange(checkoutId);
    }
  },
);

watch(activeTab, (tab) => {
  const checkoutId = props.checkout?.id;
  if (tab === "files" && checkoutId && props.repo?.kind === "git") {
    void refreshAfterGitChange(checkoutId);
  }
});

async function refreshAfterGitChange(checkoutId: string) {
  const requestGeneration = generation;
  for (const path of Object.keys(directories.value)) {
    if (requestGeneration !== generation || props.checkout?.id !== checkoutId) return;
    await loadDirectory(checkoutId, path, requestGeneration);
  }
  if (searchQuery.value.trim() && props.checkout?.id === checkoutId) {
    searchIndexCheckoutId = null;
    searchIndexPromise = null;
    searchEntries.value = [];
    searchTruncated.value = false;
    const searchRequest = ++searchGeneration;
    searchState.value = "loading";
    try {
      await loadSearchIndex(checkoutId);
      if (searchRequest === searchGeneration) searchState.value = "ready";
    } catch (error) {
      if (searchRequest === searchGeneration) {
        searchError.value = errorText(error);
        searchState.value = "error";
      }
    }
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

async function loadSearchIndex(checkoutId: string): Promise<FileSearchResult> {
  if (searchIndexCheckoutId === checkoutId) {
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
  <aside class="flex h-full w-80 shrink-0 flex-col border-l border-white/8 bg-[#15171c]">
    <div
      role="tablist"
      aria-label="Inspector sections"
      class="flex h-11 shrink-0 items-center gap-1 border-b border-white/8 px-3"
    >
      <button
        v-if="checkout"
        role="tab"
        :aria-selected="activeTab === 'files'"
        class="rounded px-3 py-2 text-[11px] font-semibold uppercase tracking-[0.12em]"
        :class="activeTab === 'files' ? 'bg-white/8 text-zinc-200' : 'text-zinc-500 hover:text-zinc-300'"
        @click="activeTab = 'files'"
      >
        Files
      </button>
      <button
        v-if="repo?.kind === 'git' && checkout && !checkout.isMissing"
        role="tab"
        :aria-selected="activeTab === 'changes'"
        class="rounded px-3 py-2 text-[11px] font-semibold uppercase tracking-[0.12em]"
        :class="activeTab === 'changes' ? 'bg-white/8 text-zinc-200' : 'text-zinc-500 hover:text-zinc-300'"
        @click="activeTab = 'changes'"
      >
        Changes
      </button>
    </div>
    <div v-show="activeTab === 'files'" class="flex min-h-0 flex-1 flex-col">
      <div class="shrink-0 border-b border-white/8 p-2">
        <input
          v-if="rootState === 'ready'"
          ref="searchInput"
          v-model="searchQuery"
          type="search"
          aria-label="Search files"
          placeholder="Search files…"
          class="h-8 w-full rounded border border-white/8 bg-[#111318] px-2 text-xs text-zinc-200 outline-none placeholder:text-zinc-600 focus:border-white/20"
        />
        <p v-else class="px-1 py-1 text-xs text-zinc-500">Checkout files</p>
      </div>
      <section class="min-h-0 flex-1 overflow-auto p-2" aria-label="Checkout files" @scroll="onTreeScroll">
        <p v-if="rootState === 'idle'" class="px-3 py-4 text-sm text-zinc-500">Open a checkout to browse files.</p>
        <p v-else-if="rootState === 'loading'" role="status" class="px-3 py-4 text-sm text-zinc-400">Loading files…</p>
        <p v-else-if="rootState === 'missing'" role="status" class="px-3 py-4 text-sm text-amber-300">
          Checkout is missing.
        </p>
        <p v-else-if="rootState === 'error'" role="alert" class="px-3 py-4 text-sm text-red-300">{{ rootError }}</p>
        <p
          v-else-if="directories['.']?.length === 0 && directoryStates['.'] !== 'truncated'"
          role="status"
          class="px-3 py-4 text-sm text-zinc-500"
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
              :style="{ paddingLeft: `${8 + item.depth * 14}px` }"
              class="flex h-8 items-center overflow-hidden"
            >
              <span v-if="!item.entry" class="truncate py-1 text-xs text-zinc-500">{{ item.message }}</span>
              <button
                v-else-if="item.entry.kind === 'directory'"
                type="button"
                class="flex h-8 w-full min-w-0 items-center truncate rounded px-2 text-left text-xs text-zinc-300 hover:bg-white/6"
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
                class="flex h-8 w-full min-w-0 items-center truncate rounded px-2 text-left text-xs hover:bg-white/6"
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
    <ChangesPane
      v-if="checkout && !checkout.isMissing && repo?.kind === 'git'"
      v-show="activeTab === 'changes'"
      :key="checkout.id"
      :checkout="checkout"
      :git-snapshot="gitSnapshot"
      :selected-path="selectedChangedPath"
      @open-change="selectChange"
    />
  </aside>
</template>
