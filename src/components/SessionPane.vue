<script setup lang="ts">
import { computed, defineComponent, h, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { PropType, VNode } from "vue";
import type { Checkout, Session, TerminalLaunchType, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import type { TerminalLaunchTarget } from "../lib/ipc";
import {
  addSessionToLayout,
  createTerminalLayout,
  findSessionTab,
  removeSessionFromLayout,
  reorderSession,
  resizeSplit,
  restoreTerminalLayout,
} from "../domain/terminal-layout";
import type { CheckoutTerminalLayout, SplitDirection, TerminalLayoutNode } from "../domain/terminal-layout";
import { loadTerminalLayout, saveTerminalLayout } from "../lib/ipc";
import Button from "./ui/button/Button.vue";
import TerminalSession from "./TerminalSession.vue";

const props = defineProps<{
  checkout: Checkout | null;
  allCheckouts?: Checkout[];
  activeSessionId: string | null;
  isOpening: boolean;
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

const emit = defineEmits<{
  openFolder: [];
  selectSession: [sessionId: string];
  workspaceUpdated: [workspace: WorkspaceState];
  sessionStatusChanged: [sessionId: string, status: TerminalSessionStatus | null];
}>();

interface TerminalView {
  key: string;
  checkoutId: string;
  session: Session | null;
  sessionType: TerminalLaunchType;
  launchTarget?: TerminalLaunchTarget;
  split?: { targetSessionId: string; direction: SplitDirection; splitId: string };
}

const layouts = ref<Record<string, CheckoutTerminalLayout>>({});
const loadedCheckoutIds = new Set<string>();
const layoutLoads = new Map<string, Promise<void>>();
const layoutSaveQueues = new Map<string, Promise<void>>();
const views = ref<TerminalView[]>([]);
const runtimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const nextViewId = ref(1);
const terminalError = ref<string | null>(null);
const selectedType = ref<TerminalLaunchType>("shell");
const draggedSessionId = ref<string | null>(null);
const isCreating = ref(false);
const checkouts = computed(() => props.allCheckouts ?? (props.checkout ? [props.checkout] : []));
const sessionsForCheckout = (checkoutId: string) =>
  checkouts.value.find((item) => item.id === checkoutId)?.sessions ?? [];
const layoutForCheckout = (checkoutId: string) => layouts.value[checkoutId];
const activeLayout = computed(() => (props.checkout ? layoutForCheckout(props.checkout.id) : undefined));
const checkoutLayoutTabs = computed(() => {
  const checkout = props.checkout;
  const layout = checkout ? layoutForCheckout(checkout.id) : undefined;
  if (!checkout || !layout) return [];
  const sessions = new Map(checkout.sessions.map((session) => [session.id, session]));
  return layout.sessionOrder.flatMap((sessionId) => {
    const session = sessions.get(sessionId);
    return session ? [session] : [];
  });
});
const activeSession = computed(
  () => props.checkout?.sessions.find((session) => session.id === props.activeSessionId) ?? null,
);
const isStarting = computed(() => isCreating.value || views.value.some((view) => view.session === null));
const activeLayoutTab = computed(() =>
  activeLayout.value?.tabs.find((tab) => tab.id === activeLayout.value?.activeTabId),
);

function saveLayout(checkoutId: string) {
  const layout = layouts.value[checkoutId];
  if (!layout) return Promise.resolve();
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

function initializeLayout(checkout: Checkout) {
  if (loadedCheckoutIds.has(checkout.id)) return Promise.resolve();
  const loading = layoutLoads.get(checkout.id);
  if (loading) return loading;
  const pending = loadTerminalLayout(checkout.id)
    .then((saved) => {
      layouts.value[checkout.id] = saved
        ? restoreTerminalLayout(saved, checkout.sessions)
        : createTerminalLayout(checkout.sessions);
    })
    .catch((cause: unknown) => {
      terminalError.value = cause instanceof Error ? cause.message : String(cause);
      layouts.value[checkout.id] = createTerminalLayout(checkout.sessions);
      return saveLayout(checkout.id);
    })
    .then(() => {
      loadedCheckoutIds.add(checkout.id);
    })
    .finally(() => layoutLoads.delete(checkout.id));
  layoutLoads.set(checkout.id, pending);
  return pending;
}

async function persistSessions(checkoutsToReconcile: Checkout[]) {
  await Promise.all(
    checkoutsToReconcile.map(async (checkout) => {
      await initializeLayout(checkout);
      const existing = layouts.value[checkout.id];
      if (!existing) return;
      const reconciled = restoreTerminalLayout(existing, checkout.sessions);
      if (JSON.stringify(reconciled) !== JSON.stringify(existing)) {
        layouts.value[checkout.id] = reconciled;
        await saveLayout(checkout.id);
      }
    }),
  );
}

watch(
  () =>
    checkouts.value
      .map((checkout) => `${checkout.id}:${checkout.sessions.map((session) => session.id).join(",")}`)
      .join("\n"),
  () => void persistSessions(checkouts.value),
  { immediate: true },
);

watch(
  () => [props.checkout?.id, props.activeSessionId, activeLayout.value?.activeTabId] as const,
  ([checkoutId, sessionId]) => {
    if (!checkoutId || !sessionId) return;
    const layout = layouts.value[checkoutId];
    const tab = layout && findSessionTab(layout, sessionId);
    if (layout && tab && layout.activeTabId !== tab.id) {
      layouts.value[checkoutId] = { ...layout, activeTabId: tab.id };
      saveLayout(checkoutId);
    }
  },
  { immediate: true },
);

function sessionLabel(session: Session) {
  const status = runtimeStatuses.value[session.id];
  if (!status) return session.status === "inactive" ? "Inactive" : "Starting";
  return status.state === "running" ? "Running" : `Exited · ${status.exitCode ?? "unknown"}`;
}

async function createTerminalSession(
  sessionType: TerminalLaunchType = selectedType.value,
  split?: TerminalView["split"],
  launchTarget?: TerminalLaunchTarget,
) {
  const checkout = props.checkout;
  if (!checkout || checkout.isMissing || isStarting.value) return;
  isCreating.value = true;
  try {
    await initializeLayout(checkout);
    if (props.checkout?.id !== checkout.id) return;
    terminalError.value = null;
    const key = `pending-${nextViewId.value++}`;
    views.value.push({ key, checkoutId: checkout.id, session: null, sessionType, launchTarget, split });
    const layout = layouts.value[checkout.id] ?? createTerminalLayout(checkout.sessions);
    const pendingSession: Session = {
      id: key,
      checkoutId: checkout.id,
      type: sessionType,
      name: sessionType === "shell" ? "Shell" : sessionType,
      createdAt: new Date().toISOString(),
      status: "inactive",
    };
    layouts.value[checkout.id] = addSessionToLayout(layout, pendingSession, split);
  } finally {
    isCreating.value = false;
  }
}

function splitActive(direction: SplitDirection) {
  const session = activeSession.value;
  if (!session || !props.checkout || props.checkout.isMissing || isStarting.value) return;
  const split = {
    targetSessionId: session.id,
    direction,
    splitId: `split:${Date.now()}:${nextViewId.value}`,
  };
  createTerminalSession("shell", split);
}

function onCreated(key: string, result: { session: Session; workspace: WorkspaceState }) {
  const view = views.value.find((item) => item.key === key);
  if (!view) return;
  view.session = result.session;
  const layout = layouts.value[view.checkoutId] ?? createTerminalLayout(sessionsForCheckout(view.checkoutId));
  const withoutPending = removeSessionFromLayout(layout, key);
  layouts.value[view.checkoutId] = addSessionToLayout(withoutPending, result.session, view.split);
  saveLayout(view.checkoutId);
  emit("workspaceUpdated", result.workspace);
}

function onStatusChanged(sessionId: string, status: TerminalSessionStatus) {
  runtimeStatuses.value[sessionId] = status;
  emit("sessionStatusChanged", sessionId, status);
}

function onClosed(key: string, workspace: WorkspaceState) {
  const view = views.value.find((item) => item.key === key);
  if (view?.session) {
    delete runtimeStatuses.value[view.session.id];
    emit("sessionStatusChanged", view.session.id, null);
  }
  views.value = views.value.filter((item) => item.key !== key);
  emit("workspaceUpdated", workspace);
}

function onFailed(key: string, message: string) {
  const view = views.value.find((item) => item.key === key);
  if (view?.session) emit("sessionStatusChanged", view.session.id, null);
  views.value = views.value.filter((item) => item.key !== key);
  if (view) {
    const layout = layouts.value[view.checkoutId];
    if (layout) {
      layouts.value[view.checkoutId] = removeSessionFromLayout(layout, key);
      saveLayout(view.checkoutId);
    }
  }
  terminalError.value = message;
}

function selectSession(session: Session) {
  const layout = props.checkout && layouts.value[props.checkout.id];
  const tab = layout && findSessionTab(layout, session.id);
  if (layout && tab && layout.activeTabId !== tab.id) {
    layouts.value[props.checkout!.id] = { ...layout, activeTabId: tab.id };
    saveLayout(props.checkout!.id);
  }
  emit("selectSession", session.id);
}

function updateSplitRatio(checkoutId: string, splitId: string, ratio: number, finished: boolean) {
  const layout = layouts.value[checkoutId];
  if (!layout) return;
  layouts.value[checkoutId] = resizeSplit(layout, splitId, ratio);
  if (finished) saveLayout(checkoutId);
}

function reorderTab(targetSessionId: string) {
  const checkoutId = props.checkout?.id;
  const sessionId = draggedSessionId.value;
  const layout = checkoutId && layouts.value[checkoutId];
  if (layout && sessionId) {
    layouts.value[checkoutId] = reorderSession(layout, sessionId, targetSessionId);
    saveLayout(checkoutId);
  }
  draggedSessionId.value = null;
}

function isEditableTarget(target: EventTarget | null) {
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement;
}

function handleKeyboard(event: KeyboardEvent) {
  if (!(event.metaKey || event.ctrlKey) || isEditableTarget(event.target) || !props.checkout) return;
  const key = event.key.toLowerCase();
  if (key === "t") {
    event.preventDefault();
    createTerminalSession("shell");
  } else if (key === "d") {
    event.preventDefault();
    splitActive(event.shiftKey ? "vertical" : "horizontal");
  }
}

watch(
  () => props.shellRequest?.token,
  async (token) => {
    if (!token || props.shellRequest?.checkoutId !== props.checkout?.id) return;
    await nextTick();
    createTerminalSession("shell");
  },
);

watch(
  () => props.nvimRequest?.token,
  async (token) => {
    const request = props.nvimRequest;
    if (!token || !request || request.checkoutId !== props.checkout?.id) return;
    await nextTick();
    const target = request.filePath
      ? { filePath: request.filePath, line: request.line ?? 1, column: request.column }
      : undefined;
    createTerminalSession("nvim", undefined, target);
  },
);

watch(
  () => props.registeredSessionIds?.join("\0"),
  (ids) => {
    if (ids === undefined) return;
    const registered = new Set(ids ? ids.split("\0") : []);
    views.value = views.value.filter((view) => {
      if (!view.session || registered.has(view.session.id)) return true;
      delete runtimeStatuses.value[view.session.id];
      return false;
    });
  },
);

onMounted(() => window.addEventListener("keydown", handleKeyboard));
onUnmounted(() => window.removeEventListener("keydown", handleKeyboard));

let SplitNode: ReturnType<typeof defineComponent>;
SplitNode = defineComponent({
  name: "TerminalSplitNode",
  props: {
    node: { type: Object as PropType<TerminalLayoutNode>, required: true },
    checkoutId: { type: String, required: true },
    active: { type: Boolean, required: true },
    activeSessionId: { type: String as PropType<string | null>, default: null },
  },
  emits: ["ratioChanged", "created", "closed", "statusChanged", "failed"],
  setup(nodeProps, { emit }) {
    function renderNode(node: TerminalLayoutNode): VNode {
      if (node.kind === "session") {
        const session = sessionsForCheckout(nodeProps.checkoutId).find((item) => item.id === node.sessionId);
        const view = views.value.find((item) => (item.session?.id ?? item.key) === node.sessionId);
        if (view) {
          return h(TerminalSession, {
            key: view.key,
            checkoutId: view.checkoutId,
            sessionType: view.sessionType,
            launchTarget: view.launchTarget,
            active: nodeProps.active,
            focused: nodeProps.activeSessionId === node.sessionId,
            onCreated: (result: { session: Session; workspace: WorkspaceState }) => onCreated(view.key, result),
            onClosed: (workspace: WorkspaceState) => onClosed(view.key, workspace),
            onStatusChanged: (status: TerminalSessionStatus) => onStatusChanged(node.sessionId, status),
            onFailed: (message: string) => onFailed(view.key, message),
          });
        }
        return h(
          "section",
          {
            class: "flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-white/8 bg-[#10151d]",
            "data-layout-session": node.sessionId,
          },
          [
            h(
              "header",
              { class: "flex h-9 shrink-0 items-center border-b border-white/8 px-3 text-xs text-zinc-400" },
              session?.name ?? "Historical session",
            ),
            h("div", { class: "grid min-h-0 flex-1 place-items-center p-5 text-center" }, [
              h("div", [
                h("p", { class: "m-0 text-sm text-zinc-300" }, "This session is inactive"),
                h(
                  "p",
                  { class: "mt-2 text-xs text-zinc-500" },
                  "Its PTY was not restored. Start a new session to continue.",
                ),
              ]),
            ]),
          ],
        );
      }

      const isHorizontal = node.direction === "horizontal";
      const resizeStart = (event: PointerEvent) => {
        event.preventDefault();
        const container = (event.currentTarget as HTMLElement).parentElement;
        if (!container) return;
        const rect = container.getBoundingClientRect();
        const extent = isHorizontal ? rect.width : rect.height;
        if (extent <= 0) return;
        const update = (move: PointerEvent, finished: boolean) => {
          const position = isHorizontal ? move.clientX - rect.left : move.clientY - rect.top;
          emit("ratioChanged", node.id, position / extent, finished);
        };
        const move = (moveEvent: PointerEvent) => update(moveEvent, false);
        const finish = (moveEvent: PointerEvent) => {
          update(moveEvent, true);
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", finish);
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", finish, { once: true });
      };
      return h(
        "div",
        {
          class: `flex h-full min-h-0 min-w-0 ${isHorizontal ? "flex-row" : "flex-col"}`,
          "data-layout-split": node.direction,
        },
        [
          h("div", { class: "min-h-0 min-w-0", style: { flex: `0 0 ${node.ratio * 100}%` } }, [
            h(SplitNode, {
              node: node.first,
              checkoutId: nodeProps.checkoutId,
              active: nodeProps.active,
              activeSessionId: nodeProps.activeSessionId,
              onRatioChanged: (id: string, ratio: number, finished: boolean) =>
                emit("ratioChanged", id, ratio, finished),
              onCreated: (result: unknown) => emit("created", result),
              onClosed: (result: unknown) => emit("closed", result),
              onStatusChanged: (result: unknown) => emit("statusChanged", result),
              onFailed: (result: unknown) => emit("failed", result),
            }),
          ]),
          h("div", {
            role: "separator",
            "aria-label": `Resize ${node.direction} terminal split`,
            "aria-orientation": isHorizontal ? "vertical" : "horizontal",
            tabindex: 0,
            class: `z-10 shrink-0 bg-white/8 hover:bg-sky-400/70 ${isHorizontal ? "w-1 cursor-col-resize" : "h-1 cursor-row-resize"}`,
            onPointerdown: resizeStart,
            onKeydown: (event: KeyboardEvent) => {
              if (
                event.key !== (isHorizontal ? "ArrowLeft" : "ArrowUp") &&
                event.key !== (isHorizontal ? "ArrowRight" : "ArrowDown")
              )
                return;
              event.preventDefault();
              const delta = event.key === (isHorizontal ? "ArrowLeft" : "ArrowUp") ? -0.05 : 0.05;
              emit("ratioChanged", node.id, node.ratio + delta, true);
            },
          }),
          h("div", { class: "min-h-0 min-w-0 flex-1" }, [
            h(SplitNode, {
              node: node.second,
              checkoutId: nodeProps.checkoutId,
              active: nodeProps.active,
              activeSessionId: nodeProps.activeSessionId,
              onRatioChanged: (id: string, ratio: number, finished: boolean) =>
                emit("ratioChanged", id, ratio, finished),
              onCreated: (result: unknown) => emit("created", result),
              onClosed: (result: unknown) => emit("closed", result),
              onStatusChanged: (result: unknown) => emit("statusChanged", result),
              onFailed: (result: unknown) => emit("failed", result),
            }),
          ]),
        ],
      );
    }

    return () => renderNode(nodeProps.node);
  },
});
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
      <div v-if="checkout" class="flex shrink-0 items-center gap-2">
        <select
          v-model="selectedType"
          aria-label="New terminal session type"
          class="h-8 rounded-md border border-white/10 bg-[#17191f] px-2 text-xs text-zinc-300"
        >
          <option value="shell">Shell</option>
          <option value="nvim">Neovim</option>
        </select>
        <span class="max-w-64 text-[10px] text-zinc-500">Server/custom deferred; use Shell for commands.</span>
        <Button :disabled="checkout.isMissing || isStarting" @click="createTerminalSession()">
          {{ isStarting ? "Starting…" : "New terminal" }}
        </Button>
        <Button
          variant="quiet"
          :disabled="!activeSession || checkout.isMissing || isStarting"
          aria-label="Split horizontally (⌘D)"
          @click="splitActive('horizontal')"
        >
          Split ↔
        </Button>
        <Button
          variant="quiet"
          :disabled="!activeSession || checkout.isMissing || isStarting"
          aria-label="Split vertically (⌘⇧D)"
          @click="splitActive('vertical')"
        >
          Split ↕
        </Button>
      </div>
    </header>

    <nav
      v-if="checkoutLayoutTabs.length || isStarting"
      class="flex shrink-0 gap-1 overflow-x-auto border-b border-white/8 px-4 py-2"
      aria-label="Checkout terminal sessions"
    >
      <button
        v-for="session in checkoutLayoutTabs"
        :key="session.id"
        type="button"
        role="tab"
        draggable="true"
        :aria-selected="session.id === activeSessionId"
        class="max-w-56 shrink-0 rounded-md border px-3 py-1.5 text-left"
        :class="
          session.id === activeSessionId
            ? 'border-white/15 bg-white/6 text-zinc-100'
            : 'border-transparent text-zinc-400 hover:bg-white/5'
        "
        @click="selectSession(session)"
        @dragstart="draggedSessionId = session.id"
        @dragover.prevent
        @drop.prevent="reorderTab(session.id)"
        @dragend="draggedSessionId = null"
      >
        <span class="block truncate text-xs">{{ session.name }}</span>
        <span class="block text-[10px] text-zinc-500">{{ session.type }} · {{ sessionLabel(session) }}</span>
      </button>
      <span
        v-if="isStarting"
        class="shrink-0 rounded-md border border-white/10 px-3 py-1.5 text-xs text-zinc-500"
        role="status"
      >
        Starting session…
      </span>
    </nav>

    <section class="relative min-h-0 flex-1 p-3" aria-label="Terminal session view">
      <template v-for="checkoutItem in checkouts" :key="checkoutItem.id">
        <div
          v-for="tab in layoutForCheckout(checkoutItem.id)?.tabs ?? []"
          v-show="checkoutItem.id === checkout?.id && tab.id === layoutForCheckout(checkoutItem.id)?.activeTabId"
          :key="`${checkoutItem.id}:${tab.id}`"
          class="absolute inset-3"
        >
          <SplitNode
            :node="tab.root"
            :checkout-id="checkoutItem.id"
            :active="checkoutItem.id === checkout?.id && tab.id === activeLayoutTab?.id"
            :active-session-id="activeSessionId"
            @ratio-changed="updateSplitRatio(checkoutItem.id, $event[0], $event[1], $event[2])"
          />
        </div>
      </template>

      <div
        v-if="!activeLayoutTab"
        class="grid h-full place-items-center rounded-lg border border-white/8 bg-[#15171c] p-8 text-center"
      >
        <div class="max-w-sm">
          <h1 class="text-base font-medium text-zinc-100">
            {{
              checkout?.isMissing
                ? "Directory missing"
                : checkout
                  ? "No terminal selected"
                  : "A workspace for focused work"
            }}
          </h1>
          <p v-if="activeSession" class="mt-2 text-sm leading-6 text-zinc-500">
            This session is historical; its process was not restored.
          </p>
          <p v-else-if="checkout" class="mt-2 break-all text-sm leading-6 text-zinc-500">{{ checkout.path }}</p>
          <p v-else class="mt-2 text-sm leading-6 text-zinc-500">
            Open a folder to get started. Marvis will keep terminals grouped by checkout.
          </p>
          <Button
            v-if="checkout && !checkout.isMissing"
            class="mt-5"
            :disabled="isStarting"
            @click="createTerminalSession('shell')"
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
