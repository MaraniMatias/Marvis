<script setup lang="ts">
import { reactive } from "vue";
import type { Checkout, Repo, TerminalSessionStatus } from "../domain/workspace";
import Button from "./ui/button/Button.vue";

defineOptions({ name: "FolderSidebar" });

const props = defineProps<{
  repos: Repo[];
  activeCheckoutId: string | null;
  activeSessionId: string | null;
  isOpening: boolean;
  activityByCheckout?: Record<string, string[]>;
  sessionRuntimeStatuses?: Record<string, TerminalSessionStatus>;
}>();

defineEmits<{
  openFolder: [];
  selectCheckout: [checkoutId: string];
  selectSession: [sessionId: string];
  locateMissing: [checkoutId: string];
  closeMissing: [checkoutId: string];
  createWorktree: [checkoutId: string];
  removeWorktree: [checkoutId: string];
}>();

const collapsedRepos = reactive(new Set<string>());

function toggleRepo(repoId: string) {
  if (collapsedRepos.has(repoId)) collapsedRepos.delete(repoId);
  else collapsedRepos.add(repoId);
}

function sessionForCheckout(checkout: Checkout) {
  return checkout.sessions.find((session) => session.id === props.activeSessionId) ?? checkout.sessions.at(-1);
}
</script>

<template>
  <aside class="flex h-full flex-col border-r border-white/8 bg-[#15171c]">
    <div class="flex items-center justify-between px-4 pb-2 pt-5">
      <h2 class="text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">Repositories</h2>
      <button
        aria-label="Open directory"
        class="grid size-7 place-items-center rounded text-lg leading-none text-zinc-400 hover:bg-white/8 hover:text-white"
        :disabled="isOpening"
        @click="$emit('openFolder')"
      >
        +
      </button>
    </div>
    <div v-if="repos.length" class="min-h-0 flex-1 overflow-y-auto px-2">
      <div v-for="repo in repos" :key="repo.id" class="mb-3">
        <div class="flex items-center gap-0.5">
          <button
            class="flex min-w-0 flex-1 items-center gap-1 rounded-md px-2 py-1.5 text-left text-xs text-zinc-300 hover:bg-white/6"
            :aria-expanded="!collapsedRepos.has(repo.id)"
            @click="toggleRepo(repo.id)"
          >
            <span class="w-3 text-[10px] text-zinc-600">{{ collapsedRepos.has(repo.id) ? "▸" : "▾" }}</span>
            <span class="min-w-0 flex-1 truncate font-medium">{{ repo.name }}</span>
            <span class="text-[10px] text-zinc-600">{{ repo.kind === "git" ? "Git" : "Plain" }}</span>
          </button>
          <button
            v-if="repo.kind === 'git' && repo.checkouts.some((checkout) => checkout.isPrimary && !checkout.isMissing)"
            :aria-label="`Create worktree for ${repo.name}`"
            title="Create worktree from main"
            class="grid size-6 shrink-0 place-items-center rounded text-sm text-zinc-500 hover:bg-white/8 hover:text-zinc-100"
            @click.stop="$emit('createWorktree', repo.checkouts.find((checkout) => checkout.isPrimary)!.id)"
          >
            +
          </button>
        </div>
        <div v-if="!collapsedRepos.has(repo.id)" class="ml-2 border-l border-white/6 pl-2">
          <div v-for="checkout in repo.checkouts" :key="checkout.id" class="mb-0.5">
            <div class="group flex items-center gap-0.5">
              <button
                class="flex min-w-0 flex-1 items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-white/6"
                :class="checkout.id === activeCheckoutId ? 'bg-white/8 text-zinc-100' : 'text-zinc-400'"
                @click="$emit('selectCheckout', checkout.id)"
              >
                <span class="truncate">
                  {{
                    checkout.isPrimary
                      ? `Primary${checkout.branch ? ` · ${checkout.branch}` : ""}`
                      : checkout.branch || checkout.path
                  }}
                </span>
                <span
                  v-if="checkout.isMissing"
                  class="shrink-0 rounded bg-amber-400/10 px-1.5 py-0.5 text-[10px] text-amber-200"
                >
                  Missing
                </span>
                <span
                  v-else-if="(activityByCheckout?.[checkout.id]?.length ?? 0) > 1"
                  class="shrink-0 rounded bg-sky-400/10 px-1.5 py-0.5 text-[10px] text-sky-200"
                  :title="`Concurrent activity: ${activityByCheckout?.[checkout.id]?.join(' + ')}`"
                  :aria-label="`Concurrent activity: ${activityByCheckout?.[checkout.id]?.join(', ')}`"
                >
                  Concurrent
                </span>
              </button>
              <button
                v-if="checkout.isMissing"
                :aria-label="`Locate ${checkout.path}`"
                title="Locate this missing directory"
                class="grid size-6 shrink-0 place-items-center rounded text-[10px] text-amber-200 hover:bg-amber-400/10"
                :disabled="isOpening"
                @click.stop="$emit('locateMissing', checkout.id)"
              >
                Locate
              </button>
              <button
                v-if="checkout.isMissing"
                :aria-label="`Close ${checkout.path}`"
                title="Close this missing location in Marvis"
                class="grid size-6 shrink-0 place-items-center rounded text-sm text-zinc-500 hover:bg-white/8 hover:text-zinc-100"
                :disabled="isOpening"
                @click.stop="$emit('closeMissing', checkout.id)"
              >
                ×
              </button>
              <button
                v-if="repo.kind === 'git' && !checkout.isPrimary && !checkout.isMissing"
                :aria-label="`Remove worktree ${checkout.branch || checkout.path}`"
                title="Remove worktree"
                class="grid size-6 shrink-0 place-items-center rounded text-sm text-zinc-500 hover:bg-red-400/10 hover:text-red-200"
                @click.stop="$emit('removeWorktree', checkout.id)"
              >
                ×
              </button>
            </div>
            <div v-if="sessionForCheckout(checkout)" class="ml-3 border-l border-white/6 pl-2">
              <button
                class="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[11px] hover:bg-white/6"
                :class="
                  sessionForCheckout(checkout)?.id === activeSessionId || checkout.id === activeCheckoutId
                    ? 'text-zinc-200'
                    : 'text-zinc-500'
                "
                :aria-label="`Terminal session: ${sessionForCheckout(checkout)?.name}`"
                :aria-current="checkout.id === activeCheckoutId ? 'page' : undefined"
                @click="$emit('selectSession', sessionForCheckout(checkout)!.id)"
              >
                <span
                  v-if="sessionRuntimeStatuses?.[sessionForCheckout(checkout)!.id]"
                  class="size-1.5 shrink-0 rounded-full"
                  :class="
                    sessionRuntimeStatuses[sessionForCheckout(checkout)!.id].state === 'running'
                      ? 'bg-green-400'
                      : 'bg-red-400'
                  "
                  role="img"
                  :aria-label="
                    sessionRuntimeStatuses[sessionForCheckout(checkout)!.id].state === 'running'
                      ? 'Session running'
                      : 'Session exited'
                  "
                />
                <span class="truncate">{{ sessionForCheckout(checkout)?.name }}</span>
                <span class="shrink-0 text-[10px] text-zinc-600">{{ sessionForCheckout(checkout)?.type }}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
    <div v-else class="px-4 py-2 text-xs leading-5 text-zinc-500">Your opened folders will appear here.</div>
    <div class="mt-auto border-t border-white/8 p-3">
      <Button variant="quiet" class="w-full justify-start" :disabled="isOpening" @click="$emit('openFolder')">
        <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" class="size-4">
          <path
            d="M2.75 5.75a1.5 1.5 0 0 1 1.5-1.5h4l1.5 1.75h6a1.5 1.5 0 0 1 1.5 1.5v6.75a1.5 1.5 0 0 1-1.5 1.5h-11a2 2 0 0 1-2-2v-8Z"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linejoin="round"
          />
        </svg>
        {{ isOpening ? "Opening…" : "Open directory" }}
      </Button>
    </div>
  </aside>
</template>
