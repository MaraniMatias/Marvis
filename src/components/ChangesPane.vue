<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { DiffFile, DiffModeEnum, DiffView } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { computed, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff, GitStatus } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import type { Checkout } from "../domain/workspace";
import { getGitDiff, getGitStatus } from "../lib/ipc";

const props = defineProps<{ checkout: Checkout; defaultBranch?: string }>();
const emit = defineEmits<{ defaultBranchUnknown: [] }>();

const status = ref<GitStatus | null>(null);
const statusState = ref<"loading" | "ready" | "error">("loading");
const statusError = ref("");
const watchError = ref("");
const selectedPath = ref<string | null>(null);
const diff = ref<GitFileDiff | null>(null);
const diffFile = shallowRef<DiffFile | null>(null);
const diffState = ref<"idle" | "loading" | "ready" | "error">("idle");
const diffError = ref("");
const hasTextHunks = computed(() => Boolean(diff.value?.patch.includes("@@")));
let statusGeneration = 0;
let diffGeneration = 0;
let unlisten: (() => void) | undefined;
let mounted = true;
let requestedDefaultBranchPrompt = false;

function errorText(error: unknown): string {
  return isIpcError(error) ? error.message : error instanceof Error ? error.message : String(error);
}

async function loadDiff(path: string) {
  const request = ++diffGeneration;
  selectedPath.value = path;
  diff.value = null;
  diffFile.value = null;
  diffError.value = "";
  diffState.value = "loading";
  try {
    const result = await getGitDiff(props.checkout.id, path);
    if (!mounted || request !== diffGeneration || selectedPath.value !== path) return;
    diff.value = result;
    if (!result.isBinary && !result.symlinkTarget && result.patch.includes("@@")) {
      const file = new DiffFile(`a/${path}`, "", `b/${path}`, "", [result.patch]);
      file.initTheme("dark");
      file.init();
      file.buildUnifiedDiffLines();
      diffFile.value = file;
    }
    diffState.value = "ready";
  } catch (error) {
    if (!mounted || request !== diffGeneration || selectedPath.value !== path) return;
    diffError.value = errorText(error);
    diffState.value = "error";
  }
}

async function loadStatus(isRefresh = false) {
  const request = ++statusGeneration;
  if (!isRefresh || !status.value) statusState.value = "loading";
  statusError.value = "";
  try {
    const result = await getGitStatus(props.checkout.id);
    if (!mounted || request !== statusGeneration) return;
    status.value = result;
    statusState.value = "ready";
    if (selectedPath.value) {
      if (result.files.some((file) => file.path === selectedPath.value)) {
        await loadDiff(selectedPath.value);
      } else {
        selectedPath.value = null;
        diff.value = null;
        diffFile.value = null;
        diffState.value = "idle";
      }
    }
  } catch (error) {
    if (!mounted || request !== statusGeneration) return;
    status.value = null;
    statusError.value = errorText(error);
    statusState.value = "error";
    if (isIpcError(error) && error.code === "default_branch_unknown" && !requestedDefaultBranchPrompt) {
      requestedDefaultBranchPrompt = true;
      emit("defaultBranchUnknown");
    }
  }
}

onMounted(async () => {
  try {
    const dispose = await listen<string>("git-status-changed", (event) => {
      if (event.payload === props.checkout.id) void loadStatus(true);
    });
    if (!mounted) dispose();
    else unlisten = dispose;
  } catch (error) {
    watchError.value = errorText(error);
  }

  if (!mounted) return;
  await loadStatus();
});

onUnmounted(() => {
  mounted = false;
  unlisten?.();
});

watch(
  () => props.defaultBranch,
  (branch, previous) => {
    if (branch && branch !== previous) {
      requestedDefaultBranchPrompt = false;
      void loadStatus(true);
    }
  },
);
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
      <p v-if="watchError" role="alert" class="mt-1 text-[11px] text-amber-300">
        Live updates unavailable: {{ watchError }}
      </p>
    </div>

    <div class="min-h-0 flex-[0_0_38%] overflow-auto border-b border-white/8 p-2" aria-label="Changed files">
      <p v-if="statusState === 'loading'" role="status" class="px-2 py-3 text-xs text-zinc-500">Loading Git status…</p>
      <p v-else-if="statusState === 'error'" role="alert" class="px-2 py-3 text-xs text-red-300">
        {{ statusError }}
      </p>
      <p v-else-if="status?.files.length === 0" role="status" class="px-2 py-3 text-xs text-zinc-500">
        No changed files.
      </p>
      <button
        v-for="file in status?.files ?? []"
        :key="file.path"
        type="button"
        class="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-white/6"
        :class="selectedPath === file.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
        @click="loadDiff(file.path)"
      >
        <span class="w-6 shrink-0 text-[10px] font-semibold text-zinc-500">{{ file.status }}</span>
        <span class="truncate font-mono">{{ file.path }}</span>
      </button>
    </div>

    <div class="min-h-0 flex-1 overflow-auto p-3" aria-label="File diff">
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
        <p v-else-if="diffState === 'ready' && !hasTextHunks" role="status" class="text-xs text-zinc-500">
          No text hunks are available for this change.
        </p>
        <DiffView
          v-else-if="diffState === 'ready' && diffFile"
          :diff-file="diffFile"
          :diff-view-mode="DiffModeEnum.Unified"
          diff-view-theme="dark"
          :diff-view-highlight="true"
          :diff-view-font-size="11"
          class="min-w-0"
        />
      </template>
    </div>
  </div>
</template>
