<script setup lang="ts">
import { Channel } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { TerminalSessionStatus } from "../domain/workspace";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import { renderPtyOutput } from "../lib/terminal-renderer";
import Button from "./ui/button/Button.vue";

const props = defineProps<{ checkoutId: string; active: boolean }>();
const emit = defineEmits<{
  created: [result: Awaited<ReturnType<typeof createTerminal>>];
  closed: [workspace: Awaited<ReturnType<typeof closeTerminal>>];
  statusChanged: [status: TerminalSessionStatus];
  failed: [message: string];
}>();

const terminalElement = ref<HTMLElement | null>(null);
const state = ref<TerminalSessionStatus>({ state: "running" });
const error = ref<string | null>(null);
const closing = ref(false);
const terminal = new Terminal({
  allowProposedApi: false,
  cursorBlink: true,
  fontFamily: "SFMono-Regular, Menlo, Monaco, monospace",
  fontSize: 13,
  scrollback: 10000,
  theme: {
    background: "#10151d",
    foreground: "#d9e2f0",
    cursor: "#7dd3fc",
    selectionBackground: "#334155",
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
    updateStatus(await getTerminalStatus(sessionId));
  } catch (cause) {
    showError(cause);
  }
}

function fitActiveView() {
  if (!props.active || !terminalElement.value) return;
  fit.fit();
}

function queueResize(cols: number, rows: number) {
  latestSize = { cols, rows };
  if (!sessionId || state.value.state !== "running") return;
  const id = sessionId;
  const resize = resizeQueue.then(() => resizeTerminal(id, cols, rows));
  resizeQueue = resize.catch(showError);
}

function queueInput(value: string) {
  if (!sessionId || closing.value || state.value.state !== "running") return;
  const id = sessionId;
  const bytes = new TextEncoder().encode(value);
  const write = inputQueue.then(() => writeTerminal(id, bytes));
  inputQueue = write.catch(showError);
}

async function requestClose() {
  if (!sessionId || closing.value) return;
  closing.value = true;
  try {
    const actualStatus = await getTerminalStatus(sessionId);
    updateStatus(actualStatus);
    if (
      actualStatus.state === "running" &&
      !window.confirm("This shell is still running. Close the session and stop its process?")
    ) {
      closing.value = false;
      return;
    }
    await inputQueue;
    await resizeQueue;
    emit("closed", await closeTerminal(sessionId));
  } catch (cause) {
    showError(cause);
    closing.value = false;
  }
}

async function startSession() {
  if (started || disposed || !props.active || !terminalElement.value) return;
  started = true;
  fitActiveView();
  channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buffer) => renderPtyOutput(terminal, buffer);
  const initialSize = { cols: terminal.cols || 80, rows: terminal.rows || 24 };
  try {
    const created = await createTerminal(props.checkoutId, initialSize.cols, initialSize.rows, channel);
    if (disposed) {
      await closeTerminal(created.session.id);
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
    terminal.focus();
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
  () => props.active,
  async (active) => {
    if (!active) return;
    await nextTick();
    fitActiveView();
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
  <section class="flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-white/8 bg-[#10151d]">
    <header class="flex h-9 shrink-0 items-center justify-between border-b border-white/8 px-3">
      <span class="truncate font-mono text-xs text-zinc-400">{{
        state.state === "running" ? "Shell running" : `Exited · code ${state.exitCode ?? "unknown"}`
      }}</span>
      <Button variant="quiet" :disabled="closing" @click="requestClose">
        {{ closing ? "Closing…" : "Close session" }}
      </Button>
    </header>
    <div ref="terminalElement" class="min-h-0 flex-1 p-2" aria-label="Shell terminal" />
    <p v-if="error" role="alert" class="m-0 border-t border-red-400/20 px-3 py-2 text-xs text-red-200">
      {{ error }}
    </p>
  </section>
</template>
