<script setup lang="ts">
import { Channel } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { TerminalLaunchType, TerminalSessionStatus } from "../domain/workspace";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import type { TerminalLaunchTarget } from "../lib/ipc";
import { renderPtyOutput } from "../lib/terminal-renderer";

const props = withDefaults(
  defineProps<{
    checkoutId: string;
    active: boolean;
    visible?: boolean;
    focused?: boolean;
    sessionType?: TerminalLaunchType;
    launchTarget?: TerminalLaunchTarget;
  }>(),
  { visible: true, focused: false, sessionType: "shell", launchTarget: undefined },
);
const emit = defineEmits<{
  created: [result: Awaited<ReturnType<typeof createTerminal>>];
  closed: [workspace: Awaited<ReturnType<typeof closeTerminal>>];
  statusChanged: [status: TerminalSessionStatus];
  failed: [message: string];
}>();

const terminalElement = ref<HTMLElement | null>(null);
const state = ref<TerminalSessionStatus>({ state: "running", foregroundProcess: false });
const error = ref<string | null>(null);
const closing = ref(false);
const terminal = new Terminal({
  allowProposedApi: false,
  cursorBlink: true,
  fontFamily: '"FiraCode Nerd Font Mono", monospace',
  fontSize: 16,
  lineHeight: 1.2,
  scrollback: 10000,
  // Only the colors come from the design tokens (B.1): the face, size and ligatures are the
  // ones the app has always had.
  theme: {
    background: "#17191f", // --marvis-bg-0
    foreground: "#d6d9e0", // --marvis-text
    cursor: "#7c9eff", // --marvis-accent
    selectionBackground: "#22252e", // --marvis-bg-2
  },
});
const fit = new FitAddon();
terminal.loadAddon(fit);

let sessionId: string | null = null;
let channel: Channel<ArrayBuffer> | undefined;
let resizeObserver: ResizeObserver | undefined;
let statusTimer: number | undefined;
let inputQueue: Promise<void> = Promise.resolve();
let resizeQueue: Promise<void> = Promise.resolve();
let resizeScheduled = false;
let disposed = false;
let started = false;
let latestSize = { cols: 0, rows: 0 };

function showError(cause: unknown) {
  error.value = cause instanceof Error ? cause.message : String(cause);
}

function updateStatus(status: TerminalSessionStatus) {
  state.value = status;
  emit("statusChanged", status);
  if (status.state === "exited" && statusTimer !== undefined) {
    window.clearInterval(statusTimer);
    statusTimer = undefined;
  }
}

async function pollStatus() {
  if (!sessionId) return;
  try {
    updateStatus(await getTerminalStatus(props.checkoutId, sessionId));
  } catch (cause) {
    showError(cause);
  }
}

function fitActiveView() {
  if (!props.active || !props.visible || !terminalElement.value) return;
  if (terminalElement.value.clientWidth === 0 || terminalElement.value.clientHeight === 0) return;
  fit.fit();
}

function queueResize(cols: number, rows: number) {
  latestSize = { cols, rows };
  if (!sessionId || state.value.state !== "running" || resizeScheduled) return;
  resizeScheduled = true;
  resizeQueue = resizeQueue
    .catch(() => {})
    .then(async () => {
      while (sessionId && state.value.state === "running") {
        const size = latestSize;
        const id = sessionId;
        await resizeTerminal(props.checkoutId, id, size.cols, size.rows);
        if (size.cols === latestSize.cols && size.rows === latestSize.rows) break;
      }
    })
    .catch(showError)
    .finally(() => {
      resizeScheduled = false;
    });
}

function queueInput(value: string) {
  if (!sessionId || closing.value || state.value.state !== "running") return;
  const id = sessionId;
  const bytes = new TextEncoder().encode(value);
  const write = inputQueue.then(() => writeTerminal(props.checkoutId, id, bytes));
  inputQueue = write.catch(showError);
}

async function requestClose() {
  if (!sessionId || closing.value) return false;
  closing.value = true;
  try {
    const actualStatus = await getTerminalStatus(props.checkoutId, sessionId);
    updateStatus(actualStatus);
    if (
      actualStatus.state === "running" &&
      !window.confirm("This terminal session is still running. Close the session and stop its process?")
    ) {
      closing.value = false;
      return false;
    }
    await inputQueue;
    await resizeQueue;
    emit("closed", await closeTerminal(props.checkoutId, sessionId));
    return true;
  } catch (cause) {
    showError(cause);
    closing.value = false;
    return false;
  }
}

defineExpose({ requestClose, focus: () => terminal.focus() });

async function startSession() {
  if (started || disposed || !props.active || !terminalElement.value) return;
  started = true;
  fitActiveView();
  channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buffer) => renderPtyOutput(terminal, buffer);
  const initialSize = { cols: terminal.cols || 80, rows: terminal.rows || 24 };
  try {
    const created = await createTerminal(
      props.checkoutId,
      initialSize.cols,
      initialSize.rows,
      props.sessionType,
      channel,
      props.launchTarget,
    );
    if (disposed) {
      await closeTerminal(props.checkoutId, created.session.id);
      return;
    }
    sessionId = created.session.id;
    emit("created", created);
    if (
      latestSize.cols &&
      latestSize.rows &&
      (latestSize.cols !== initialSize.cols || latestSize.rows !== initialSize.rows)
    ) {
      queueResize(terminal.cols, terminal.rows);
    }
    if (props.visible && props.active && props.focused) terminal.focus();
    await pollStatus();
    if (state.value.state === "running") {
      statusTimer = window.setInterval(() => void pollStatus(), 750);
    }
  } catch (cause) {
    showError(cause);
    emit("failed", error.value ?? "Could not start terminal");
  }
}

terminal.onData(queueInput);
terminal.onResize(({ cols, rows }) => queueResize(cols, rows));

watch(
  () => [props.active, props.visible, props.focused] as const,
  async ([active, visible, focused]) => {
    if (!active) return;
    await nextTick();
    fitActiveView();
    if (visible && focused) terminal.focus();
    void startSession();
  },
);

onMounted(() => {
  if (!terminalElement.value) return;
  terminal.open(terminalElement.value);
  fitActiveView();
  resizeObserver = new ResizeObserver(() => fitActiveView());
  resizeObserver.observe(terminalElement.value);
  void startSession();
});

onUnmounted(() => {
  disposed = true;
  if (statusTimer !== undefined) window.clearInterval(statusTimer);
  resizeObserver?.disconnect();
  if (channel) channel.onmessage = () => {};
  terminal.dispose();
});
</script>

<template>
  <section class="terminal-surface flex h-full min-h-0 flex-col overflow-hidden">
    <div ref="terminalElement" class="terminal-host min-h-0 flex-1" aria-label="Shell terminal" />
    <p v-if="error" role="alert" class="m-0 border-t border-(--marvis-border) px-3 py-2 text-xs text-(--marvis-red)">
      {{ error }}
    </p>
  </section>
</template>
