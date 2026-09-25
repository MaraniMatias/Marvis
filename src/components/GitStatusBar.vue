<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { watch, ref } from "vue";
import { isIpcError } from "../domain/ipc";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";
import { getGitStatus, unwatchGitCheckout, watchGitCheckout } from "../lib/ipc";

const props = defineProps<{ checkout: Checkout | null; repo: Repo | null }>();
const emit = defineEmits<{ defaultBranchUnknown: [] }>();

const status = ref<GitStatus | null>(null);
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
    let requestedDefaultBranch = false;
    let statusRequest = 0;
    onCleanup(() => {
      current = false;
      unlisten?.();
      if (watching && checkoutId) void unwatchGitCheckout(checkoutId).catch(() => undefined);
    });

    status.value = null;
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
    </template>
    <span v-else-if="loading" role="status">Loading Git status…</span>
    <span v-else-if="error" role="status" :title="error">Git status unavailable</span>
    <span v-else role="status">Git status unavailable</span>
    <span v-if="watchError && checkout && !checkout.isMissing && repo?.kind === 'git'" class="ml-auto text-amber-300">
      Live updates unavailable
    </span>
  </footer>
</template>
