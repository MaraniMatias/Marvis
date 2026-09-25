<script setup lang="ts">
/* eslint-disable vue/html-self-closing, vue/html-indent, vue/html-closing-bracket-newline */
import { computed, ref } from "vue";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";

const props = defineProps<{
  checkout: Checkout;
  gitSnapshot: ActiveGitSnapshot;
  selectedPath: string | null;
}>();
const emit = defineEmits<{ openChange: [value: { checkoutId: string; path: string }] }>();

const query = ref("");
const listScrollTop = ref(0);
const status = computed(() => props.gitSnapshot.status);
const statusState = computed(() => props.gitSnapshot.statusState);
const statusError = computed(() => props.gitSnapshot.changesStatusError);
const watchError = computed(() => props.gitSnapshot.changesWatchError);
const viewedPaths = computed(() => props.gitSnapshot.viewedPaths);
const viewedError = computed(() => props.gitSnapshot.viewedError);
const viewedCount = computed(() => {
  const viewed = new Set(viewedPaths.value);
  return status.value?.files.filter((file) => viewed.has(file.path)).length ?? 0;
});
const filteredFiles = computed(() => {
  const normalized = query.value.trim().toLocaleLowerCase();
  return (status.value?.files ?? []).filter((file) =>
    normalized ? `${file.path} ${file.oldPath ?? ""}`.toLocaleLowerCase().includes(normalized) : true,
  );
});
const visibleFileWindow = computed(() => {
  const rowHeight = 28;
  const windowSize = 80;
  const start = Math.min(
    Math.max(0, filteredFiles.value.length - windowSize),
    Math.max(0, Math.floor(listScrollTop.value / rowHeight) - 8),
  );
  const end = Math.min(filteredFiles.value.length, start + windowSize);
  return {
    files: filteredFiles.value.slice(start, end),
    paddingTop: start * rowHeight,
    paddingBottom: (filteredFiles.value.length - end) * rowHeight,
  };
});

function onScroll(event: Event) {
  listScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
}

function openChange(path: string) {
  emit("openChange", { checkoutId: props.checkout.id, path });
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="shrink-0 border-b border-white/8 px-3 py-3">
      <p class="truncate text-xs font-medium text-zinc-200">
        {{ status?.branch || (status?.head ? `HEAD ${status.head}` : "Git changes") }}
      </p>
      <p class="mt-1 text-[11px] text-zinc-500">
        <template v-if="status">vs {{ status.defaultBranch }} · {{ status.files.length }} changed</template>
        <template v-else>Comparing with the default branch</template>
      </p>
      <p v-if="status" class="mt-1 text-[11px] text-zinc-500">
        {{ status.aheadCount }} {{ status.aheadCount === 1 ? "commit" : "commits" }} ahead · {{ viewedCount }}/{{
          status.files.length
        }}
        viewed
      </p>
      <p v-if="viewedError" role="alert" class="mt-1 text-[10px] text-amber-300">Viewed progress: {{ viewedError }}</p>
      <p v-if="watchError" role="alert" class="mt-1 text-[11px] text-amber-300">
        Live updates unavailable: {{ watchError }}
      </p>
    </div>
    <div class="shrink-0 border-b border-white/8 p-2">
      <input
        v-model="query"
        type="search"
        aria-label="Filter changed files"
        placeholder="Filter changed files…"
        class="h-8 w-full rounded border border-white/8 bg-black/10 px-2 text-xs text-zinc-200 outline-none placeholder:text-zinc-600 focus:border-sky-400/50"
      />
    </div>
    <div class="min-h-0 flex-1 overflow-auto p-2" aria-label="Changed files" @scroll="onScroll">
      <p v-if="statusState === 'loading'" role="status" class="px-2 py-3 text-xs text-zinc-500">Loading Git status…</p>
      <p v-else-if="statusState === 'error'" role="alert" class="px-2 py-3 text-xs text-red-300">
        {{ statusError || gitSnapshot.statusError }}
      </p>
      <p v-else-if="status?.files.length === 0" role="status" class="px-2 py-3 text-xs text-zinc-500">
        No changed files.
      </p>
      <p v-else-if="filteredFiles.length === 0" role="status" class="px-2 py-3 text-xs text-zinc-500">
        No changed files match this filter.
      </p>
      <div
        v-else
        :style="{
          paddingTop: `${visibleFileWindow.paddingTop}px`,
          paddingBottom: `${visibleFileWindow.paddingBottom}px`,
        }"
      >
        <button
          v-for="file in visibleFileWindow.files"
          :key="file.path"
          type="button"
          class="flex h-7 w-full items-center gap-2 rounded px-2 text-left text-[12px] hover:bg-white/6"
          :class="selectedPath === file.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
          @click="openChange(file.path)"
        >
          <span class="w-6 shrink-0 text-[10px] font-semibold text-zinc-500">{{ file.status }}</span>
          <span class="truncate font-mono" :title="file.oldPath ? `${file.oldPath} → ${file.path}` : file.path">
            <template v-if="file.oldPath"
              ><span class="text-zinc-600">{{ file.oldPath }} → </span></template
            >{{ file.path }}
          </span>
          <span v-if="viewedPaths.includes(file.path)" class="ml-auto shrink-0 text-[10px] text-green-400">Viewed</span>
        </button>
      </div>
    </div>
  </div>
</template>
