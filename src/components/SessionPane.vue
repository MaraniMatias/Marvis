<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { Plus as PlusIcon } from "@lucide/vue";
import type { Checkout, Session, TerminalLaunchType, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import {
  addSessionToLayout,
  createTerminalLayout,
  normalizeTerminalLayout,
  removeSessionFromLayout,
} from "../domain/terminal-layout";
import type { CheckoutTerminalLayout } from "../domain/terminal-layout";
import type { TerminalLaunchTarget } from "../lib/ipc";
import { loadTerminalLayout, saveTerminalLayout } from "../lib/ipc";
import { useToasts } from "../presentation/toasts";
import Button from "./ui/button/Button.vue";
import TerminalSession from "./TerminalSession.vue";

const props = defineProps<{
  checkout: Checkout | null;
  activeSessionId: string | null;
  isOpening: boolean;
  visible?: boolean;
  shellRequest?: { checkoutId: string; token: number } | null;
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
  focus(): void;
}

const layouts = ref<Record<string, CheckoutTerminalLayout>>({});
const loadedCheckoutIds = new Set<string>();
const layoutLoads = new Map<string, Promise<void>>();
const layoutSaveQueues = new Map<string, Promise<void>>();
const views = ref<TerminalView[]>([]);
const pendingViewKey = ref<string | null>(null);
const startingCheckoutIds = ref(new Set<string>());
const terminalRefs = new Map<string, TerminalSessionHandle>();
const nextViewId = ref(1);
const { push: pushToast } = useToasts();
const checkout = computed(() => props.checkout);
const activeView = computed(() => {
  const checkoutViews = views.value.filter((view) => view.checkoutId === checkout.value?.id);
  return (
    checkoutViews.find((view) => view.key === pendingViewKey.value) ??
    checkoutViews.find((view) => view.session?.id === props.activeSessionId) ??
    checkoutViews.at(-1)
  );
});
const isStarting = computed(
  () =>
    Boolean(checkout.value && startingCheckoutIds.value.has(checkout.value.id)) ||
    Boolean(activeView.value && !activeView.value.session),
);

/**
 * A failure to read or write the layout is not shown at all: a stale layout is healed into a
 * working one, so reporting it would name a problem the user never has. Everything else is
 * announced (A.6) rather than left in a bar under the terminal, which pushed the last lines of
 * output out of the panel for a message that is gone six seconds later.
 */
function reportTerminalError(cause: unknown) {
  const message = cause instanceof Error ? cause.message : String(cause);
  if (/^(?:saved terminal layout is invalid|terminal layout\b)/.test(message)) return;
  pushToast(message);
}

function saveLayout(checkoutId: string, layout: CheckoutTerminalLayout) {
  layouts.value[checkoutId] = layout;
  const snapshot = JSON.parse(JSON.stringify(layout)) as CheckoutTerminalLayout;
  const previous = layoutSaveQueues.get(checkoutId) ?? Promise.resolve();
  const pending = previous
    .catch(() => {})
    .then(() => saveTerminalLayout(checkoutId, snapshot))
    .catch(reportTerminalError);
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
      reportTerminalError(cause);
      layouts.value[target.id] = createTerminalLayout([]);
    })
    .then(() => {
      loadedCheckoutIds.add(target.id);
    })
    .finally(() => layoutLoads.delete(target.id));
  layoutLoads.set(target.id, pending);
  return pending;
}

async function createTerminalSession(
  sessionType: TerminalLaunchType = "shell",
  launchTarget?: TerminalLaunchTarget,
  additional = false,
) {
  const target = props.checkout;
  if (!target || target.isMissing || (!additional && views.value.some((view) => view.checkoutId === target.id))) return;
  if (startingCheckoutIds.value.has(target.id)) return;
  startingCheckoutIds.value = new Set(startingCheckoutIds.value).add(target.id);
  try {
    await initializeLayout(target);
    if (props.checkout?.id !== target.id || target.isMissing) return;
    const key = `pending-${nextViewId.value++}`;
    pendingViewKey.value = key;
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

function focusActiveTerminal() {
  const view = activeView.value;
  if (view) terminalRefs.get(view.key)?.focus();
}

async function requestClose(sessionId: string) {
  const view = views.value.find((item) => item.session?.id === sessionId);
  if (!view) return false;
  return (await terminalRefs.get(view.key)?.requestClose()) ?? false;
}

defineExpose({ focusActiveTerminal, requestClose });

function onCreated(key: string, result: { session: Session; workspace: WorkspaceState }) {
  const view = views.value.find((item) => item.key === key);
  if (!view || view.session) return;
  view.session = result.session;
  const layout = layouts.value[view.checkoutId] ?? createTerminalLayout([]);
  void saveLayout(view.checkoutId, addSessionToLayout(layout, result.session));
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
    if (pendingViewKey.value === key) pendingViewKey.value = null;
    void saveLayout(
      view.checkoutId,
      removeSessionFromLayout(layouts.value[view.checkoutId] ?? createTerminalLayout([]), view.session.id),
    );
  }
  views.value = views.value.filter((item) => item.key !== key);
  emit("workspaceUpdated", workspace);
}

function onFailed(key: string, message: string) {
  const view = views.value.find((item) => item.key === key);
  terminalRefs.delete(key);
  if (view?.session) {
    emit("sessionStatusChanged", view.session.id, null);
    void saveLayout(
      view.checkoutId,
      removeSessionFromLayout(layouts.value[view.checkoutId] ?? createTerminalLayout([]), view.session.id),
    );
  }
  if (pendingViewKey.value === key) pendingViewKey.value = null;
  views.value = views.value.filter((item) => item.key !== key);
  pushToast(message);
}

watch(
  () => props.activeSessionId,
  (sessionId) => {
    if (sessionId && views.value.some((view) => view.session?.id === sessionId)) pendingViewKey.value = null;
  },
);

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
  () => props.shellRequest?.token,
  (token) => {
    if (token && props.shellRequest?.checkoutId === props.checkout?.id)
      void createTerminalSession("shell", undefined, true);
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
  <main class="session-pane flex min-w-0 flex-1 flex-col">
    <section class="relative min-h-0 flex-1" aria-label="Terminal session view">
      <div
        v-for="view in views"
        v-show="view.checkoutId === checkout?.id && view.key === activeView?.key"
        :key="view.key"
        class="absolute inset-0"
      >
        <TerminalSession
          :ref="(instance) => setTerminalRef(view.key, instance)"
          :key="view.key"
          :checkout-id="view.checkoutId"
          :session-type="view.sessionType"
          :launch-target="view.launchTarget"
          :active="view.checkoutId === checkout?.id && view.key === activeView?.key"
          :visible="isVisible"
          :focused="isVisible && view.session?.id === activeSessionId"
          @created="onCreated(view.key, $event)"
          @closed="onClosed(view.key, $event)"
          @status-changed="onStatusChanged(view.session?.id ?? view.key, $event)"
          @failed="onFailed(view.key, $event)"
        />
      </div>

      <div v-if="!activeView" class="session-empty grid h-full place-items-center p-6 text-center">
        <div class="max-w-md">
          <p class="text-sm text-(--marvis-text-dim)">
            {{
              checkout?.isMissing
                ? `Directory missing: ${checkout.path}`
                : checkout
                  ? isStarting
                    ? "Starting terminal…"
                    : "No terminal session."
                  : "Open a folder to start a terminal."
            }}
          </p>
          <Button
            v-if="checkout && !checkout.isMissing && !isStarting"
            class="mt-4"
            @click="createTerminalSession('shell')"
          >
            <PlusIcon class="size-3.5 shrink-0" aria-hidden="true" />
            New terminal
          </Button>
          <Button v-else-if="!checkout" class="mt-4" :disabled="isOpening" @click="$emit('openFolder')">
            <PlusIcon class="size-3.5 shrink-0" aria-hidden="true" />
            {{ isOpening ? "Opening…" : "Open directory" }}
          </Button>
          <span v-else-if="isStarting" class="mt-3 block text-xs text-(--marvis-text-faint)" role="status">
            Starting session…
          </span>
        </div>
      </div>
    </section>
  </main>
</template>
