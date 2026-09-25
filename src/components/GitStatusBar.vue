<script setup lang="ts">
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { computed } from "vue";

const props = defineProps<{
  checkout: Checkout | null;
  repo: Repo | null;
  gitSnapshot: ActiveGitSnapshot;
  concurrentActors?: string[];
}>();
const status = computed(() => props.gitSnapshot.status);
const loading = computed(() => props.gitSnapshot.loading);
const statusError = computed(() => props.gitSnapshot.statusError);

const viewedCount = computed(() => {
  const viewed = new Set(props.gitSnapshot.viewedPaths);
  return status.value?.files.filter((file) => viewed.has(file.path)).length ?? 0;
});
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
    <span v-else-if="statusError" role="status" :title="statusError">Git status unavailable</span>
    <span v-else role="status">Git status unavailable</span>
    <span
      v-if="gitSnapshot.watchError && checkout && !checkout.isMissing && repo?.kind === 'git'"
      class="ml-auto text-amber-300"
    >
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
