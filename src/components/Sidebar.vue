<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { computed, ref } from "vue";
import type { Checkout, Repo, TerminalSessionStatus } from "../domain/workspace";

defineOptions({ name: "FolderSidebar" });

const props = withDefaults(
  defineProps<{
    repos: Repo[];
    activeCheckoutId: string | null;
    activeSessionId: string | null;
    isOpening: boolean;
    activityByCheckout?: Record<string, string[]>;
    sessionRuntimeStatuses?: Record<string, TerminalSessionStatus>;
    collapsedRepoIds?: string[];
  }>(),
  {
    collapsedRepoIds: () => [],
    activityByCheckout: () => ({}),
    sessionRuntimeStatuses: () => ({}),
  },
);

const emit = defineEmits<{
  openFolder: [];
  selectCheckout: [checkoutId: string];
  selectSession: [sessionId: string];
  locateMissing: [checkoutId: string];
  closeMissing: [checkoutId: string];
  createWorktree: [checkoutId: string];
  newTerminal: [checkoutId: string];
  removeWorktree: [checkoutId: string];
  closeSession: [sessionId: string];
  updateCollapsedRepos: [repoIds: string[]];
}>();
const repoQuery = ref("");
const filteredRepos = computed(() => {
  const query = repoQuery.value.trim().toLocaleLowerCase();
  return query ? props.repos.filter((repo) => repo.name.toLocaleLowerCase().includes(query)) : props.repos;
});

function toggleRepo(repoId: string) {
  const next = new Set(props.collapsedRepoIds);
  if (next.has(repoId)) next.delete(repoId);
  else next.add(repoId);
  emit("updateCollapsedRepos", [...next]);
}

function sessionState(sessionId: string, sessionStatus: "active" | "inactive") {
  return props.sessionRuntimeStatuses[sessionId]?.state ?? (sessionStatus === "active" ? "running" : "exited");
}

function checkoutLabel(checkout: Checkout) {
  return checkout.isPrimary
    ? `Primary${checkout.branch ? ` · ${checkout.branch}` : ""}`
    : checkout.branch || checkout.path;
}
</script>

<template>
  <aside class="app-sidebar flex h-full min-h-0 flex-col border-r">
    <div class="sidebar-toolbar flex h-10 shrink-0 items-center gap-1 border-b px-2">
      <input
        v-model="repoQuery"
        type="search"
        aria-label="Search repositories"
        placeholder="Filter repositories…"
        class="sidebar-filter h-7 min-w-0 flex-1 rounded px-2 text-xs outline-none placeholder:text-zinc-600"
      />
      <button
        type="button"
        aria-label="Open directory"
        title="Open directory"
        class="grid size-7 shrink-0 place-items-center rounded-sm text-base leading-none text-zinc-400 hover:bg-white/8 hover:text-white focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
        :disabled="isOpening"
        @click="emit('openFolder')"
      >
        +
      </button>
    </div>
    <div v-if="filteredRepos.length" class="min-h-0 flex-1 overflow-y-auto px-1.5 py-1">
      <div v-for="repo in filteredRepos" :key="repo.id" class="mb-1">
        <div class="group/repo flex h-7 items-center rounded-sm hover:bg-white/5">
          <button
            type="button"
            class="flex h-full min-w-0 flex-1 items-center gap-1 rounded-sm px-1.5 text-left text-[13px] text-zinc-300 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
            :aria-expanded="!collapsedRepoIds.includes(repo.id)"
            :title="repo.name"
            @click="toggleRepo(repo.id)"
          >
            <span class="w-3 shrink-0 text-[10px] text-zinc-600">{{
              collapsedRepoIds.includes(repo.id) ? "▸" : "▾"
            }}</span>
            <span class="min-w-0 flex-1 truncate font-medium">{{ repo.name }}</span>
            <span class="shrink-0 text-[10px] text-zinc-600">{{ repo.kind === "git" ? "Git" : "Plain" }}</span>
          </button>
          <button
            v-if="repo.kind === 'git' && repo.checkouts.some((checkout) => checkout.isPrimary && !checkout.isMissing)"
            type="button"
            :aria-label="`Create worktree for ${repo.name}`"
            title="Create worktree from main"
            class="grid size-6 shrink-0 place-items-center rounded-sm text-sm text-zinc-500 hover:bg-white/8 hover:text-zinc-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
            @click.stop="emit('createWorktree', repo.checkouts.find((checkout) => checkout.isPrimary)!.id)"
          >
            <svg aria-hidden="true" viewBox="0 0 16 16" class="size-3.5 fill-none stroke-current" stroke-width="1.3">
              <path d="M4 2v12M4 4h5a3 3 0 1 1 0 6H4" />
              <path d="M11.5 2.5v4m-2-2h4" />
            </svg>
          </button>
        </div>
        <div v-if="!collapsedRepoIds.includes(repo.id)" class="ml-2 border-l border-white/6 pl-1.5">
          <div v-for="checkout in repo.checkouts" :key="checkout.id" class="mb-0.5">
            <div
              class="group/checkout flex h-7 items-center gap-0.5 rounded-sm border-l-2 border-transparent"
              :class="
                checkout.id === activeCheckoutId
                  ? 'sidebar-checkout-active border-sky-300/65 bg-sky-400/10 text-zinc-100 hover:bg-sky-400/15'
                  : 'text-zinc-400 hover:bg-white/6'
              "
            >
              <button
                type="button"
                class="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-sm px-1.5 text-left text-[13px] focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
                :aria-current="checkout.id === activeCheckoutId ? 'page' : undefined"
                :title="checkout.path"
                @click="emit('selectCheckout', checkout.id)"
              >
                <span aria-hidden="true" class="shrink-0 text-[10px] text-zinc-600">›</span>
                <span class="min-w-0 flex-1 truncate">{{ checkoutLabel(checkout) }}</span>
                <span
                  v-if="checkout.isMissing"
                  class="shrink-0 rounded bg-amber-400/10 px-1 py-0.5 text-[10px] text-amber-200"
                >
                  Missing
                </span>
                <span
                  v-else-if="(activityByCheckout?.[checkout.id]?.length ?? 0) > 1"
                  class="shrink-0 rounded bg-sky-400/10 px-1 py-0.5 text-[10px] text-sky-200"
                  :title="`Concurrent activity: ${activityByCheckout?.[checkout.id]?.join(' + ')}`"
                  :aria-label="`Concurrent activity: ${activityByCheckout?.[checkout.id]?.join(', ')}`"
                >
                  Concurrent
                </span>
              </button>
              <button
                v-if="checkout.isMissing"
                type="button"
                :aria-label="`Locate ${checkout.path}`"
                title="Locate this missing directory"
                class="flex h-6 shrink-0 items-center gap-1 rounded-sm px-1 text-[10px] text-amber-200 hover:bg-amber-400/10 focus-visible:outline focus-visible:outline-1 focus-visible:outline-amber-300"
                :disabled="isOpening"
                @click.stop="emit('locateMissing', checkout.id)"
              >
                <svg aria-hidden="true" viewBox="0 0 16 16" class="size-3 fill-none stroke-current" stroke-width="1.4">
                  <circle cx="7" cy="7" r="4.5" />
                  <path d="m10.5 10.5 3 3M7 4.5v5M4.5 7h5" />
                </svg>
                Locate
              </button>
              <button
                v-if="checkout.isMissing"
                type="button"
                :aria-label="`Close ${checkout.path}`"
                title="Close this missing location in Marvis"
                class="grid size-6 shrink-0 place-items-center rounded-sm text-sm text-zinc-500 hover:bg-white/8 hover:text-zinc-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
                :disabled="isOpening"
                @click.stop="emit('closeMissing', checkout.id)"
              >
                <svg aria-hidden="true" viewBox="0 0 16 16" class="size-3.5 stroke-current" stroke-width="1.4">
                  <path d="m4 4 8 8M12 4l-8 8" />
                </svg>
              </button>
              <template v-else>
                <button
                  type="button"
                  :aria-label="`New terminal for ${checkoutLabel(checkout)}`"
                  title="New terminal"
                  class="grid size-6 shrink-0 place-items-center rounded-sm text-zinc-500 opacity-0 transition-opacity group-hover/checkout:opacity-100 group-focus-within/checkout:opacity-100 hover:bg-white/8 hover:text-zinc-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
                  @click.stop="emit('newTerminal', checkout.id)"
                >
                  <svg
                    aria-hidden="true"
                    viewBox="0 0 16 16"
                    class="size-3.5 fill-none stroke-current"
                    stroke-width="1.3"
                  >
                    <rect x="2" y="3" width="12" height="10" rx="1.5" />
                    <path d="m4.5 6 2 2-2 2M8 10h3" />
                  </svg>
                </button>
                <button
                  v-if="repo.kind === 'git' && !checkout.isPrimary"
                  type="button"
                  :aria-label="`Remove worktree ${checkout.branch || checkout.path}`"
                  title="Remove worktree"
                  class="grid size-6 shrink-0 place-items-center rounded-sm text-zinc-500 opacity-0 transition-opacity group-hover/checkout:opacity-100 group-focus-within/checkout:opacity-100 hover:bg-red-400/10 hover:text-red-200 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-red-300"
                  @click.stop="emit('removeWorktree', checkout.id)"
                >
                  <svg aria-hidden="true" viewBox="0 0 16 16" class="size-3.5 stroke-current" stroke-width="1.4">
                    <path d="m4 4 8 8M12 4l-8 8" />
                  </svg>
                </button>
              </template>
            </div>
            <div
              v-for="session in checkout.sessions"
              :key="session.id"
              class="group/session ml-3 flex h-7 items-center rounded-sm"
              :class="session.id === activeSessionId ? 'bg-sky-400/10 hover:bg-sky-400/15' : 'hover:bg-white/6'"
            >
              <button
                type="button"
                class="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-sm px-1.5 text-left text-[13px] focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
                :class="session.id === activeSessionId ? 'text-sky-100' : 'text-zinc-500 hover:text-zinc-300'"
                :aria-label="`Terminal session: ${session.name}`"
                :aria-current="session.id === activeSessionId ? 'page' : undefined"
                :title="session.name"
                @click="emit('selectSession', session.id)"
              >
                <span
                  class="size-1.5 shrink-0 rounded-full"
                  :class="sessionState(session.id, session.status) === 'running' ? 'bg-green-400' : 'bg-red-400'"
                  role="img"
                  :aria-label="
                    sessionState(session.id, session.status) === 'running' ? 'Session running' : 'Session exited'
                  "
                />
                <span class="min-w-0 flex-1 truncate">{{ session.name }}</span>
                <span class="shrink-0 text-[10px] text-zinc-600">{{ session.type }}</span>
              </button>
              <button
                type="button"
                :aria-label="`Close terminal session: ${session.name}`"
                title="Close terminal session"
                class="grid size-6 shrink-0 place-items-center rounded-sm text-zinc-500 opacity-0 transition-opacity group-hover/session:opacity-100 group-focus-within/session:opacity-100 hover:bg-white/8 hover:text-zinc-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400"
                @click.stop="emit('closeSession', session.id)"
              >
                <svg aria-hidden="true" viewBox="0 0 16 16" class="size-3.5 stroke-current" stroke-width="1.4">
                  <path d="m4 4 8 8M12 4l-8 8" />
                </svg>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
    <div v-else-if="repoQuery" role="status" class="px-4 py-3 text-xs leading-5 text-zinc-500">
      No matching repositories.
    </div>
    <div v-else class="px-4 py-3 text-xs leading-5 text-zinc-500">
      Your opened folders will appear here. Use + to add one.
    </div>
  </aside>
</template>
