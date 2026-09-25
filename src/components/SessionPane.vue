<script setup lang="ts">
import { computed, ref } from "vue";
import type { Checkout, Session, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import Button from "./ui/button/Button.vue";
import TerminalSession from "./TerminalSession.vue";

const props = defineProps<{
  checkout: Checkout | null;
  activeSessionId: string | null;
  isOpening: boolean;
}>();

const emit = defineEmits<{
  openFolder: [];
  selectSession: [sessionId: string];
  workspaceUpdated: [workspace: WorkspaceState];
}>();

interface TerminalView {
  key: string;
  checkoutId: string;
  session: Session | null;
}

const views = ref<TerminalView[]>([]);
const runtimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const pendingViewKey = ref<string | null>(null);
const nextViewId = ref(1);
const terminalError = ref<string | null>(null);
const activeView = computed(() =>
  views.value.find(
    (view) =>
      view.checkoutId === props.checkout?.id &&
      (view.session?.id === props.activeSessionId || (!view.session && view.key === pendingViewKey.value)),
  ),
);
const selectedSession = computed(
  () => props.checkout?.sessions.find((session) => session.id === props.activeSessionId) ?? null,
);
const isStarting = computed(() => views.value.some((view) => view.session === null));

function isViewActive(view: TerminalView) {
  return activeView.value?.key === view.key;
}

function sessionLabel(session: Session) {
  const status = runtimeStatuses.value[session.id];
  if (!status) return "Inactive";
  return status.state === "running" ? "Running" : `Exited · ${status.exitCode ?? "unknown"}`;
}

function createTerminalSession() {
  const checkout = props.checkout;
  if (!checkout || checkout.isMissing || isStarting.value) return;
  terminalError.value = null;
  const key = `pending-${nextViewId.value++}`;
  views.value.push({ key, checkoutId: checkout.id, session: null });
  pendingViewKey.value = key;
}

function onCreated(key: string, result: { session: Session; workspace: WorkspaceState }) {
  const view = views.value.find((item) => item.key === key);
  if (view) view.session = result.session;
  pendingViewKey.value = null;
  emit("workspaceUpdated", result.workspace);
}

function onStatusChanged(view: TerminalView, status: TerminalSessionStatus) {
  if (view.session) runtimeStatuses.value[view.session.id] = status;
}

function onClosed(key: string, workspace: WorkspaceState) {
  const view = views.value.find((item) => item.key === key);
  if (view?.session) delete runtimeStatuses.value[view.session.id];
  views.value = views.value.filter((item) => item.key !== key);
  emit("workspaceUpdated", workspace);
}

function onFailed(key: string, message: string) {
  views.value = views.value.filter((item) => item.key !== key);
  if (pendingViewKey.value === key) pendingViewKey.value = null;
  terminalError.value = message;
}

function selectSession(sessionId: string) {
  pendingViewKey.value = null;
  emit("selectSession", sessionId);
}
</script>

<template>
  <main class="flex min-w-0 flex-1 flex-col bg-[#111318]">
    <header class="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-white/8 px-6">
      <div v-if="checkout" class="flex min-w-0 items-center gap-2 text-sm">
        <span class="truncate text-zinc-200">{{ checkout.path.split(/[\\/]/).filter(Boolean).at(-1) }}</span>
        <span class="text-zinc-600">/</span>
        <span class="text-zinc-500">Sessions</span>
      </div>
      <span v-else class="text-sm text-zinc-500">Sessions</span>
      <Button v-if="checkout" :disabled="checkout.isMissing || isStarting" @click="createTerminalSession">
        {{ isStarting ? "Starting…" : "New terminal" }}
      </Button>
    </header>

    <nav
      v-if="checkout?.sessions.length || isStarting"
      class="flex shrink-0 gap-1 overflow-x-auto border-b border-white/8 px-4 py-2"
      aria-label="Checkout terminal sessions"
    >
      <button
        v-for="session in checkout?.sessions ?? []"
        :key="session.id"
        type="button"
        role="tab"
        :aria-selected="session.id === activeSessionId"
        class="max-w-56 shrink-0 rounded-md border px-3 py-1.5 text-left"
        :class="
          session.id === activeSessionId
            ? 'border-white/15 bg-white/6 text-zinc-100'
            : 'border-transparent text-zinc-400 hover:bg-white/5'
        "
        @click="selectSession(session.id)"
      >
        <span class="block truncate text-xs">{{ session.name }}</span>
        <span class="block text-[10px] text-zinc-500">{{ session.type }} · {{ sessionLabel(session) }}</span>
      </button>
      <span
        v-if="isStarting"
        class="shrink-0 rounded-md border border-white/10 px-3 py-1.5 text-xs text-zinc-500"
        role="status"
      >
        Starting shell…
      </span>
    </nav>

    <section class="relative min-h-0 flex-1 p-3" aria-label="Terminal session view">
      <div v-for="view in views" v-show="isViewActive(view)" :key="view.key" class="absolute inset-3">
        <TerminalSession
          :checkout-id="view.checkoutId"
          :active="isViewActive(view)"
          @created="onCreated(view.key, $event)"
          @closed="onClosed(view.key, $event)"
          @status-changed="onStatusChanged(view, $event)"
          @failed="onFailed(view.key, $event)"
        />
      </div>

      <div
        v-if="!activeView"
        class="grid h-full place-items-center rounded-lg border border-white/8 bg-[#15171c] p-8 text-center"
      >
        <div class="max-w-sm">
          <h1 class="text-base font-medium text-zinc-100">
            {{
              selectedSession
                ? "This session is inactive"
                : checkout?.isMissing
                  ? "Directory missing"
                  : checkout
                    ? "No terminal selected"
                    : "A workspace for focused work"
            }}
          </h1>
          <p v-if="selectedSession" class="mt-2 text-sm leading-6 text-zinc-500">
            This session is historical; its process was not restored. Start a new shell to continue working.
          </p>
          <p v-else-if="checkout" class="mt-2 break-all text-sm leading-6 text-zinc-500">
            {{ checkout.path }}
          </p>
          <p v-else class="mt-2 text-sm leading-6 text-zinc-500">
            Open a folder to get started. Marvis will keep terminals grouped by checkout.
          </p>
          <Button
            v-if="checkout && !checkout.isMissing"
            class="mt-5"
            :disabled="isStarting"
            @click="createTerminalSession"
          >
            {{ isStarting ? "Starting…" : "New terminal" }}
          </Button>
          <Button v-else-if="!checkout" class="mt-5" :disabled="isOpening" @click="$emit('openFolder')">
            {{ isOpening ? "Opening…" : "Open directory" }}
          </Button>
        </div>
      </div>
    </section>

    <p v-if="terminalError" role="alert" class="m-0 border-t border-red-400/20 px-5 py-3 text-sm text-red-200">
      {{ terminalError }}
    </p>
  </main>
</template>
