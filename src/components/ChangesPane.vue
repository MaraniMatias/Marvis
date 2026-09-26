<script setup lang="ts">
/* eslint-disable vue/html-self-closing, vue/html-indent, vue/html-closing-bracket-newline */
import { computed, nextTick, ref, watch } from "vue";
import type { Checkout } from "../domain/workspace";
import { agentAttention } from "../domain/agent";
import type { AgentSession } from "../domain/agent";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";

const props = defineProps<{
  checkout: Checkout;
  gitSnapshot: ActiveGitSnapshot;
  review?: Pick<ActiveReviewNotes, "notes" | "rounds" | "markSent">;
  /** Live agent sessions of this checkout, so the round can pick a target. */
  agentSessions?: AgentSession[];
  agentTargetId?: string | null;
  selectedPath: string | null;
  scrollTop: number;
}>();
const emit = defineEmits<{
  openChange: [value: { checkoutId: string; path: string }];
  scrollPositionChanged: [top: number];
  sendReview: [ids: string[]];
  selectAgentTarget: [sessionId: string];
}>();

const query = ref("");
const listScrollTop = ref(props.scrollTop);
const listViewport = ref<HTMLElement | null>(null);
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

function statusClass(status: string): string {
  switch (status) {
    case "A":
      return "changes-status-added";
    case "M":
      return "changes-status-modified";
    case "D":
      return "changes-status-removed";
    case "R":
    case "C":
      return "changes-status-renamed";
    default:
      return "changes-status-default";
  }
}
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
  emit("scrollPositionChanged", listScrollTop.value);
}

watch(
  () => [props.checkout.id, statusState.value] as const,
  async ([checkoutId, state]) => {
    if (state !== "ready") return;
    const savedTop = props.scrollTop;
    listScrollTop.value = savedTop;
    await nextTick();
    if (props.checkout.id === checkoutId && listViewport.value) listViewport.value.scrollTop = savedTop;
  },
  { immediate: true, flush: "post" },
);

function openChange(path: string) {
  emit("openChange", { checkoutId: props.checkout.id, path });
}

const noteCount = computed(() => props.review?.notes.length ?? 0);
const draftCount = computed(() => props.review?.notes.filter((note) => note.status === "draft").length ?? 0);
const outdatedCount = computed(() => props.review?.notes.filter((note) => note.outdated).length ?? 0);
const sendingReview = ref(false);
/** Only shown when the choice is real: one live agent is not a choice. */
const multipleAgents = computed(() => (props.agentSessions ?? []).length > 1);
const pendingRoundCount = computed(
  () => (props.review?.rounds ?? []).filter((round) => round.status !== "acked").length,
);
const blockedAgent = computed(() =>
  (props.agentSessions ?? []).some((session) => agentAttention(session) === "blocked"),
);
function agentLabel(session: AgentSession): string {
  const attention = agentAttention(session);
  if (attention === "blocked") return `${session.title} (needs permission)`;
  if (attention === "busy") return `${session.title} (working)`;
  return session.title;
}
/** Outdated notes are held back unless the user opts in, so nothing stale reaches the agent. */
const includeOutdated = ref(false);
const sendableNotes = computed(() =>
  (props.review?.notes ?? []).filter((note) => includeOutdated.value || !note.outdated),
);

async function sendReviewToAgent() {
  const notes = sendableNotes.value;
  if (sendingReview.value || notes.length === 0) return;
  sendingReview.value = true;
  try {
    const ids = notes.map((note) => note.id);
    await props.review?.markSent(ids);
    emit("sendReview", ids);
  } finally {
    sendingReview.value = false;
  }
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="shrink-0 border-b p-1.5">
      <input
        v-model="query"
        type="search"
        aria-label="Filter changed files"
        placeholder="Filter changed files…"
        class="inspector-filter h-7 w-full rounded-sm px-2 text-xs outline-none placeholder:text-zinc-600"
      />
    </div>
    <div class="changes-summary shrink-0 border-b px-2 py-2">
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
      <div v-if="noteCount > 0" class="mt-2 flex items-center gap-2">
        <button
          type="button"
          class="rounded-sm border border-sky-500/40 bg-sky-500/10 px-2 py-0.5 text-[11px] text-sky-200 disabled:opacity-50"
          :disabled="sendingReview || sendableNotes.length === 0"
          data-testid="send-review"
          @click="sendReviewToAgent"
        >
          {{ sendingReview ? "Sending…" : "Send to agent" }}
        </button>
        <span class="text-[10px] text-zinc-500" data-testid="review-note-count">
          {{ noteCount }} {{ noteCount === 1 ? "note" : "notes"
          }}<template v-if="draftCount > 0"> · {{ draftCount }} draft</template>
        </span>
      </div>
      <label v-if="outdatedCount > 0" class="mt-1 flex items-center gap-1.5 text-[10px] text-amber-400">
        <input v-model="includeOutdated" type="checkbox" data-testid="include-outdated" />
        {{ outdatedCount }} outdated {{ outdatedCount === 1 ? "note" : "notes" }} left out
      </label>
      <div v-if="multipleAgents" class="mt-1 flex items-center gap-1.5 text-[10px] text-zinc-500">
        <label for="review-agent-target">Send to</label>
        <select
          id="review-agent-target"
          :value="agentTargetId ?? undefined"
          data-testid="review-agent-target"
          class="min-w-0 flex-1 rounded-sm border border-white/10 bg-black/30 px-1 py-0.5 text-[10px] text-zinc-200 outline-none"
          @change="emit('selectAgentTarget', ($event.target as HTMLSelectElement).value)"
        >
          <option v-for="session in agentSessions" :key="session.id" :value="session.id">
            {{ agentLabel(session) }}
          </option>
        </select>
      </div>
      <p v-if="blockedAgent" role="alert" class="mt-1 text-[10px] text-amber-300">
        An agent asked for a permission this OpenCode version cannot answer, so its turn is waiting.
      </p>
      <p v-if="pendingRoundCount > 0" class="mt-1 text-[10px] text-zinc-500" data-testid="pending-rounds">
        {{ pendingRoundCount }} {{ pendingRoundCount === 1 ? "round" : "rounds" }} not finished
      </p>
    </div>
    <div ref="listViewport" class="min-h-0 flex-1 overflow-auto p-1" aria-label="Changed files" @scroll="onScroll">
      <p v-if="statusState === 'loading'" role="status" class="pane-state text-xs">Loading Git status…</p>
      <p v-else-if="statusState === 'error'" role="alert" class="pane-state text-xs text-red-300">
        {{ statusError || gitSnapshot.statusError }}
      </p>
      <p v-else-if="status?.files.length === 0" role="status" class="pane-state text-xs">No changed files.</p>
      <p v-else-if="filteredFiles.length === 0" role="status" class="pane-state text-xs">
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
          class="changes-file-row flex h-7 w-full items-center gap-2 rounded-sm px-2 text-left text-[12px]"
          :class="selectedPath === file.path ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
          @click="openChange(file.path)"
        >
          <span
            class="grid size-4 shrink-0 place-items-center rounded-sm text-[10px] font-semibold"
            :class="statusClass(file.status)"
            :title="`Change status: ${file.status}`"
          >
            {{ file.status }}
          </span>
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
