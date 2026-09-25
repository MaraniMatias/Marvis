<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from "vue";
import { isIpcError } from "../domain/ipc";
import type { FileEntry } from "../domain/files";
import type { Checkout, Repo } from "../domain/workspace";
import { listCheckoutFiles, readCheckoutFile } from "../lib/ipc";

const ChangesPane = defineAsyncComponent(() => import("./ChangesPane.vue"));

const props = defineProps<{ checkout: Checkout | null; repo?: Repo | null }>();
defineEmits<{ defaultBranchUnknown: [] }>();

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
const selectedPath = ref<string | null>(null);
const content = ref("");
const contentState = ref<"idle" | "loading" | "ready" | "error">("idle");
const contentError = ref("");
const activeTab = ref<"files" | "changes">("files");
let generation = 0;

function errorText(error: unknown): string {
  if (isIpcError(error)) {
    switch (error.code) {
      case "folder_missing":
        return "File or folder no longer exists.";
      case "permission_denied":
        return "Permission denied while reading this folder or file.";
      case "file_too_large":
        return "This file is larger than the 1 MiB text limit.";
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
    selectedPath.value = null;
    content.value = "";
    contentState.value = "idle";
    contentError.value = "";
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

async function selectFile(entry: FileEntry) {
  const checkoutId = props.checkout?.id;
  if (!checkoutId) return;
  const requestGeneration = generation;
  selectedPath.value = entry.path;
  content.value = "";
  contentError.value = "";
  contentState.value = "loading";
  try {
    const result = await readCheckoutFile(checkoutId, entry.path);
    if (requestGeneration !== generation || selectedPath.value !== entry.path) return;
    content.value = result.content;
    contentState.value = "ready";
  } catch (error) {
    if (requestGeneration !== generation || selectedPath.value !== entry.path) return;
    contentError.value = errorText(error);
    contentState.value = "error";
  }
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
</script>

<template>
  <aside class="flex h-full w-80 shrink-0 flex-col border-l border-white/8 bg-[#15171c]">
    <div role="group" aria-label="Inspector sections" class="flex h-14 items-center gap-1 border-b border-white/8 px-3">
      <button
        :aria-pressed="activeTab === 'files'"
        class="rounded px-3 py-2 text-[11px] font-semibold uppercase tracking-[0.12em]"
        :class="activeTab === 'files' ? 'bg-white/8 text-zinc-200' : 'text-zinc-500 hover:text-zinc-300'"
        @click="activeTab = 'files'"
      >
        Files
      </button>
      <button
        v-if="repo?.kind === 'git'"
        :aria-pressed="activeTab === 'changes'"
        class="rounded px-3 py-2 text-[11px] font-semibold uppercase tracking-[0.12em]"
        :class="activeTab === 'changes' ? 'bg-white/8 text-zinc-200' : 'text-zinc-500 hover:text-zinc-300'"
        @click="activeTab = 'changes'"
      >
        Changes
      </button>
    </div>
    <div v-if="activeTab === 'files'" class="flex min-h-0 flex-1 flex-col">
      <section class="min-h-0 flex-1 overflow-auto border-b border-white/8 p-2" aria-label="Checkout files">
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
          <p v-if="directoryStates['.'] === 'truncated'" class="px-2 py-1 text-xs text-zinc-500">
            Some entries omitted (folder is large).
          </p>
          <div
            v-for="(item, index) in visibleEntries"
            :key="item.entry?.path ?? `${item.depth}-${index}-${item.message}`"
            :style="{ paddingLeft: `${8 + item.depth * 14}px` }"
            class="flex min-h-8 items-center"
          >
            <span v-if="!item.entry" class="py-1 text-xs text-zinc-500">{{ item.message }}</span>
            <button
              v-else-if="item.entry.kind === 'directory'"
              type="button"
              class="w-full truncate rounded px-2 py-1 text-left text-xs text-zinc-300 hover:bg-white/6"
              :aria-expanded="expanded.includes(item.entry.path)"
              @click="toggleDirectory(item.entry)"
            >
              <span class="mr-2 text-zinc-500">
                {{ expanded.includes(item.entry.path) ? "▾" : "▸" }}
              </span>
              <span>{{ item.entry.name }}</span>
            </button>
            <button
              v-else
              type="button"
              class="w-full truncate rounded px-2 py-1 text-left text-xs hover:bg-white/6"
              :class="selectedPath === item.entry.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
              @click="selectFile(item.entry)"
            >
              <span class="mr-2 text-zinc-600">
                {{ item.entry.kind === "symlink" ? "↗" : "·" }}
              </span>
              <span>{{ item.entry.name }}</span>
            </button>
          </div>
        </template>
      </section>
      <section class="min-h-0 flex-1 overflow-auto p-4" aria-label="Selected file">
        <p v-if="!selectedPath" class="text-sm text-zinc-500">Select a file to read it.</p>
        <template v-else>
          <p class="mb-3 break-all font-mono text-[11px] text-zinc-400">{{ selectedPath }}</p>
          <p v-if="contentState === 'loading'" role="status" class="text-sm text-zinc-400">Loading file…</p>
          <p v-else-if="contentState === 'error'" role="alert" class="text-sm text-amber-300">{{ contentError }}</p>
          <p v-else-if="contentState === 'ready' && content.length === 0" role="status" class="text-sm text-zinc-500">
            This file is empty.
          </p>
          <pre
            v-else-if="contentState === 'ready'"
            class="whitespace-pre-wrap break-words font-mono text-xs leading-5 text-zinc-300"
          >
            {{ content }}
          </pre>
        </template>
      </section>
    </div>
    <ChangesPane
      v-else-if="checkout && repo?.kind === 'git'"
      :key="checkout.id"
      :checkout="checkout"
      :default-branch="repo.defaultBranch"
      @default-branch-unknown="$emit('defaultBranchUnknown')"
    />
  </aside>
</template>
