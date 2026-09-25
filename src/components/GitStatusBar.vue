<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { computed, watch, ref } from "vue";
import { isIpcError } from "../domain/ipc";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";
import { getGitStatus, getGitViewedFiles, unwatchGitCheckout, watchGitCheckout } from "../lib/ipc";

const props = defineProps<{
  checkout: Checkout | null;
  repo: Repo | null;
  concurrentActors?: string[];
}>();
const emit = defineEmits<{ defaultBranchUnknown: [] }>();

const status = ref<GitStatus | null>(null);
const viewedPaths = ref<string[]>([]);
const viewedCount = computed(() => {
  const viewed = new Set(viewedPaths.value);
  return status.value?.files.filter((file) => viewed.has(file.path)).length ?? 0;
});
const loading = ref(false);
const error = ref("");
const watchError = ref("");
let generation = 0;

function errorText(cause: unknown): string {
  return isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}

watch(
  () => [props.checkout?.id, props.checkout?.isMissing, props.repo?.kind, props.repo?.defaultBranch] as const,
  async ([checkoutId, isMissing, repoKind], _previous, onCleanup) => {
    const requestGeneration = ++generation;
    let current = true;
    let watching = false;
    let unlisten: (() => void) | undefined;
    let unlistenViewed: (() => void) | undefined;
    let requestedDefaultBranch = false;
    let statusRequest = 0;
    let viewedRequest = 0;
    onCleanup(() => {
      current = false;
      unlisten?.();
      unlistenViewed?.();
      if (watching && checkoutId) void unwatchGitCheckout(checkoutId).catch(() => undefined);
    });

    status.value = null;
    viewedPaths.value = [];
    error.value = "";
    watchError.value = "";
    loading.value = false;
    if (!checkoutId) return;
    if (isMissing) {
      error.value = "Directory missing";
      return;
    }
    if (repoKind !== "git") return;

    loading.value = true;
    const refresh = async () => {
      const refreshRequest = ++statusRequest;
      try {
        const result = await getGitStatus(checkoutId);
        if (!current || requestGeneration !== generation || refreshRequest !== statusRequest) return;
        status.value = result;
        error.value = "";
        const viewedGeneration = ++viewedRequest;
        try {
          const viewed = await getGitViewedFiles(checkoutId);
          if (
            current &&
            requestGeneration === generation &&
            refreshRequest === statusRequest &&
            viewedGeneration === viewedRequest
          ) {
            viewedPaths.value = viewed;
          }
        } catch {
          if (current && viewedGeneration === viewedRequest) viewedPaths.value = [];
        }
      } catch (cause) {
        if (!current || requestGeneration !== generation || refreshRequest !== statusRequest) return;
        status.value = null;
        error.value = errorText(cause);
        if (isIpcError(cause) && cause.code === "default_branch_unknown" && !requestedDefaultBranch) {
          requestedDefaultBranch = true;
          emit("defaultBranchUnknown");
        }
      } finally {
        if (current && requestGeneration === generation && refreshRequest === statusRequest) loading.value = false;
      }
    };

    try {
      const dispose = await listen<string>("git-status-changed", (event) => {
        if (event.payload === checkoutId) void refresh();
      });
      if (!current) {
        dispose();
        return;
      }
      unlisten = dispose;
    } catch (cause) {
      if (current) watchError.value = errorText(cause);
    }

    try {
      const dispose = await listen<string>("git-viewed-changed", (event) => {
        if (event.payload === checkoutId) void refresh();
      });
      if (!current) {
        dispose();
        return;
      }
      unlistenViewed = dispose;
    } catch (cause) {
      if (current) watchError.value = errorText(cause);
    }

    try {
      await watchGitCheckout(checkoutId);
      if (!current) {
        void unwatchGitCheckout(checkoutId).catch(() => undefined);
        return;
      }
      watching = true;
    } catch (cause) {
      if (current) watchError.value = errorText(cause);
    }
    if (current) await refresh();
  },
  { immediate: true },
);

const branchLabel = () => status.value?.branch ?? (status.value?.head ? `HEAD ${status.value.head}` : "Detached HEAD");
</script>

<template>
  <footer
    class="flex h-8 shrink-0 items-center gap-3 border-t border-white/8 bg-[#15171c] px-4 text-[11px] text-zinc-400"
  >
    <span v-if="!checkout" role="status">No checkout selected</span>
    <span v-else-if="checkout.isMissing" role="status">Directory missing</span>
    <span v-else-if="repo?.kind !== 'git'" role="status">Plain directory · Git status unavailable</span>
    <template v-else-if="status">
      <span role="status" aria-label="Git branch">{{ branchLabel() }}</span>
      <span role="status" aria-label="Changed files">{{ status.files.length }} changed</span>
      <span role="status" aria-label="Commits ahead">
        {{ status.aheadCount }} {{ status.aheadCount === 1 ? "commit" : "commits" }} ahead
      </span>
      <span role="status" aria-label="Viewed files">{{ viewedCount }}/{{ status.files.length }} viewed</span>
    </template>
    <span v-else-if="loading" role="status">Loading Git status…</span>
    <span v-else-if="error" role="status" :title="error">Git status unavailable</span>
    <span v-else role="status">Git status unavailable</span>
    <span v-if="watchError && checkout && !checkout.isMissing && repo?.kind === 'git'" class="ml-auto text-amber-300">
      Live updates unavailable
    </span>
    <span
      v-if="concurrentActors && concurrentActors.length > 1"
      role="status"
      class="ml-auto truncate text-sky-200"
      :title="`Observed concurrent activity: ${concurrentActors.join(' + ')}. External editors and agents are not tracked.`"
      aria-label="Concurrent activity"
    >
      Concurrent · {{ concurrentActors.join(" + ") }}
    </span>
  </footer>
</template>
