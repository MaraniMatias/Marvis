<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { Checkout, Session, TerminalLaunchType, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import { createTerminalLayout, normalizeTerminalLayout } from "../domain/terminal-layout";
import type { CheckoutTerminalLayout } from "../domain/terminal-layout";
import type { TerminalLaunchTarget } from "../lib/ipc";
import { loadTerminalLayout, saveTerminalLayout } from "../lib/ipc";
import Button from "./ui/button/Button.vue";
import TerminalSession from "./TerminalSession.vue";

const props = defineProps<{
  checkout: Checkout | null;
  activeSessionId: string | null;
  isOpening: boolean;
  visible?: boolean;
  shellRequest?: { checkoutId: string; token: number } | null;
  nvimRequest?: {
    checkoutId: string;
    filePath?: string;
    line?: number;
    column?: number;
    token: number;
  } | null;
  registeredSessionIds?: string[];
}>();
const isVisible = computed(() => props.visible ?? true);

const emit = defineEmits<{
  openFolder: [];
  workspaceUpdated: [workspace: WorkspaceState];
  sessionStatusChanged: [sessionId: string, status: TerminalSessionStatus | null];
}>();

interface TerminalView {
  /** Stable for the entire lifetime of this view; never replace it with the backend session ID. */
  key: string;
  checkoutId: string;
  session: Session | null;
  sessionType: TerminalLaunchType;
  launchTarget?: TerminalLaunchTarget;
}
interface TerminalSessionHandle {
  requestClose(): Promise<boolean>;
}

const layouts = ref<Record<string, CheckoutTerminalLayout>>({});
const loadedCheckoutIds = new Set<string>();
const layoutLoads = new Map<string, Promise<void>>();
const layoutSaveQueues = new Map<string, Promise<void>>();
const views = ref<TerminalView[]>([]);
const startingCheckoutIds = ref(new Set<string>());
const nvimLaunchingCheckoutIds = ref(new Set<string>());
const handledNvimRequestTokens = new Set<number>();
const terminalRefs = new Map<string, TerminalSessionHandle>();
const nextViewId = ref(1);
const terminalError = ref<string | null>(null);
const checkout = computed(() => props.checkout);
const activeView = computed(() => views.value.find((view) => view.checkoutId === checkout.value?.id));
const isStarting = computed(
  () =>
    Boolean(checkout.value && startingCheckoutIds.value.has(checkout.value.id)) ||
    Boolean(activeView.value && !activeView.value.session),
);

function saveLayout(checkoutId: string, layout: CheckoutTerminalLayout) {
  layouts.value[checkoutId] = layout;
  const snapshot = JSON.parse(JSON.stringify(layout)) as CheckoutTerminalLayout;
  const previous = layoutSaveQueues.get(checkoutId) ?? Promise.resolve();
  const pending = previous
    .catch(() => {})
    .then(() => saveTerminalLayout(checkoutId, snapshot))
    .catch((cause: unknown) => {
      terminalError.value = cause instanceof Error ? cause.message : String(cause);
    });
  layoutSaveQueues.set(checkoutId, pending);
  return pending;
}

function initializeLayout(target: Checkout) {
  if (loadedCheckoutIds.has(target.id)) return Promise.resolve();
  const loading = layoutLoads.get(target.id);
  if (loading) return loading;
  const pending = loadTerminalLayout(target.id)
    .then((saved) => {
      const normalized = normalizeTerminalLayout(saved, target.sessions, props.activeSessionId);
      layouts.value[target.id] = normalized;
      if (saved && JSON.stringify(saved) !== JSON.stringify(normalized)) {
        return saveLayout(target.id, normalized);
      }
    })
    .catch((cause: unknown) => {
      terminalError.value = cause instanceof Error ? cause.message : String(cause);
      layouts.value[target.id] = createTerminalLayout([]);
    })
    .then(() => {
      loadedCheckoutIds.add(target.id);
    })
    .finally(() => layoutLoads.delete(target.id));
  layoutLoads.set(target.id, pending);
  return pending;
}

async function createTerminalSession(sessionType: TerminalLaunchType = "shell", launchTarget?: TerminalLaunchTarget) {
  const target = props.checkout;
  if (!target || target.isMissing || views.value.some((view) => view.checkoutId === target.id)) return;
  if (startingCheckoutIds.value.has(target.id)) return;
  startingCheckoutIds.value = new Set(startingCheckoutIds.value).add(target.id);
  try {
    await initializeLayout(target);
    if (props.checkout?.id !== target.id || target.isMissing) return;
    terminalError.value = null;
    const key = `pending-${nextViewId.value++}`;
    views.value.push({ key, checkoutId: target.id, session: null, sessionType, launchTarget });
  } finally {
    const pending = new Set(startingCheckoutIds.value);
    pending.delete(target.id);
    startingCheckoutIds.value = pending;
  }
}

function setTerminalRef(key: string, instance: unknown) {
  if (instance) terminalRefs.set(key, instance as TerminalSessionHandle);
  else terminalRefs.delete(key);
}

async function launchNeovim(target?: TerminalLaunchTarget) {
  const selectedCheckout = props.checkout;
  if (!selectedCheckout || selectedCheckout.isMissing || nvimLaunchingCheckoutIds.value.has(selectedCheckout.id))
    return;
  nvimLaunchingCheckoutIds.value = new Set(nvimLaunchingCheckoutIds.value).add(selectedCheckout.id);
  try {
    const currentView = views.value.find((view) => view.checkoutId === selectedCheckout.id);
    if (!currentView) {
      await createTerminalSession("nvim", target);
      return;
    }
    if (!currentView.session) {
      terminalError.value = "Wait for the current terminal to start before opening Neovim.";
      return;
    }
    if (currentView.sessionType === "nvim") return;
    const terminal = terminalRefs.get(currentView.key);
    if (!terminal) {
      terminalError.value = "The current terminal is not ready to open Neovim.";
      return;
    }
    if ((await terminal.requestClose()) && props.checkout?.id === selectedCheckout.id) {
      await createTerminalSession("nvim", target);
    }
  } finally {
    const pending = new Set(nvimLaunchingCheckoutIds.value);
    pending.delete(selectedCheckout.id);
    nvimLaunchingCheckoutIds.value = pending;
  }
}

function handleNvimRequest(request: NonNullable<typeof props.nvimRequest>) {
  if (handledNvimRequestTokens.has(request.token)) return false;
  handledNvimRequestTokens.add(request.token);
  const target = request.filePath
    ? { filePath: request.filePath, line: request.line ?? 1, column: request.column }
    : undefined;
  void launchNeovim(target);
  return true;
}

function onCreated(key: string, result: { session: Session; workspace: WorkspaceState }) {
  const view = views.value.find((item) => item.key === key);
  if (!view || view.session) return;
  view.session = result.session;
  void saveLayout(view.checkoutId, createTerminalLayout([result.session]));
  emit("workspaceUpdated", result.workspace);
}

function onStatusChanged(sessionId: string, status: TerminalSessionStatus) {
  emit("sessionStatusChanged", sessionId, status);
}

function onClosed(key: string, workspace: WorkspaceState) {
  const view = views.value.find((item) => item.key === key);
  if (view?.session) {
    terminalRefs.delete(key);
    emit("sessionStatusChanged", view.session.id, null);
    void saveLayout(view.checkoutId, createTerminalLayout([]));
  }
  views.value = views.value.filter((item) => item.key !== key);
  emit("workspaceUpdated", workspace);
}

function onFailed(key: string, message: string) {
  const view = views.value.find((item) => item.key === key);
  terminalRefs.delete(key);
  if (view?.session) emit("sessionStatusChanged", view.session.id, null);
  views.value = views.value.filter((item) => item.key !== key);
  if (view) void saveLayout(view.checkoutId, createTerminalLayout([]));
  terminalError.value = message;
}

function isEditableTarget(target: EventTarget | null) {
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement;
}

function handleKeyboard(event: KeyboardEvent) {
  if (!(event.metaKey || event.ctrlKey) || isEditableTarget(event.target) || !props.checkout) return;
  if (event.key.toLowerCase() === "t") {
    event.preventDefault();
    void createTerminalSession("shell");
  }
}

watch(
  () => [props.checkout?.id, props.checkout?.isMissing, props.isOpening] as const,
  ([checkoutId, isMissing, isOpening]) => {
    if (!checkoutId || isMissing || isOpening) return;
    const request = props.nvimRequest;
    if (request?.checkoutId === checkoutId && handleNvimRequest(request)) return;
    void createTerminalSession("shell");
  },
  { immediate: true },
);

watch(
  () => props.shellRequest?.token,
  (token) => {
    if (token && props.shellRequest?.checkoutId === props.checkout?.id) void createTerminalSession("shell");
  },
);

watch(
  () => props.nvimRequest?.token,
  () => {
    const request = props.nvimRequest;
    if (!request || request.checkoutId !== props.checkout?.id) return;
    handleNvimRequest(request);
  },
);

watch(
  () => props.registeredSessionIds?.join("\0"),
  (ids) => {
    if (ids === undefined) return;
    const registered = new Set(ids ? ids.split("\0") : []);
    views.value = views.value.filter((view) => {
      if (!view.session || registered.has(view.session.id)) return true;
      terminalRefs.delete(view.key);
      return false;
    });
  },
);

onMounted(() => window.addEventListener("keydown", handleKeyboard));
onUnmounted(() => window.removeEventListener("keydown", handleKeyboard));
</script>

<template>
  <main class="flex min-w-0 flex-1 flex-col bg-[#111318]">
    <section class="relative min-h-0 flex-1 p-3" aria-label="Terminal session view">
      <div v-for="view in views" v-show="view.checkoutId === checkout?.id" :key="view.key" class="absolute inset-3">
        <TerminalSession
          :ref="(instance) => setTerminalRef(view.key, instance)"
          :key="view.key"
          :checkout-id="view.checkoutId"
          :session-type="view.sessionType"
          :launch-target="view.launchTarget"
          :active="view.checkoutId === checkout?.id"
          :visible="isVisible"
          :focused="isVisible && view.session?.id === activeSessionId"
          @created="onCreated(view.key, $event)"
          @closed="onClosed(view.key, $event)"
          @status-changed="onStatusChanged(view.session?.id ?? view.key, $event)"
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
              checkout?.isMissing
                ? "Directory missing"
                : checkout
                  ? isStarting
                    ? "Starting terminal…"
                    : "No terminal"
                  : "A workspace for focused work"
            }}
          </h1>
          <p v-if="checkout" class="mt-2 break-all text-sm leading-6 text-zinc-500">{{ checkout.path }}</p>
          <p v-else class="mt-2 text-sm leading-6 text-zinc-500">
            Open a folder to get started. Marvis starts a terminal for the selected checkout.
          </p>
          <Button
            v-if="checkout && !checkout.isMissing && !isStarting"
            class="mt-5"
            @click="createTerminalSession('shell')"
          >
            New terminal
          </Button>
          <Button v-else-if="!checkout" class="mt-5" :disabled="isOpening" @click="$emit('openFolder')">
            {{ isOpening ? "Opening…" : "Open directory" }}
          </Button>
          <span v-else-if="isStarting" class="mt-5 block text-xs text-zinc-500" role="status">Starting session…</span>
        </div>
      </div>
    </section>

    <p v-if="terminalError" role="alert" class="m-0 border-t border-red-400/20 px-5 py-3 text-sm text-red-200">
      {{ terminalError }}
    </p>
  </main>
</template>
