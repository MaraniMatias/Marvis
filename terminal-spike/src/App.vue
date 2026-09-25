<script setup lang="ts">
import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import "./style.css";
import { nextTick, onMounted, onUnmounted, ref } from "vue";
import { TooltipContent, TooltipProvider, TooltipRoot, TooltipTrigger } from "reka-ui";
import Button from "./components/ui/Button.vue";
import Input from "./components/ui/Input.vue";
import { exactPayloadCommand } from "./payload-command.js";

interface BackendMetrics {
  bytesRead: number;
  bytesSent: number;
  droppedBytes: number;
  readerClosed: boolean;
}

interface PayloadProbe {
  begin: Uint8Array;
  end: Uint8Array;
  carry: Uint8Array;
  started: boolean;
  startedAt: number;
  bytes: number;
  expectedBytes: number;
  invalidBytes: number;
}

interface PingProbe {
  marker: Uint8Array;
  carry: Uint8Array;
  startedAt: number;
}

const yesLine = Uint8Array.of(121, 13, 10);
const yesStart = Uint8Array.of(13, 10, 121, 13, 10);

const cwd = ref("");
const payloadSize = ref("16");
const status = ref("Starting renderer…");
const metricsText = ref("No PTY started.");
const resizeText = ref("Resize: — · UI frame gap: —");
const shellRunning = ref(false);
const startingShell = ref(false);
const fiveMode = ref(false);
const fiveOpening = ref(false);
const terminalElement = ref<HTMLElement | null>(null);
const fivePaneContents: Array<HTMLElement | null> = [];
const terminal = new Terminal({
  allowProposedApi: false,
  cursorBlink: true,
  fontFamily: "SFMono-Regular, Menlo, Monaco, monospace",
  fontSize: 13,
  scrollback: 5000,
  theme: {
    background: "#10151d",
    foreground: "#d9e2f0",
    cursor: "#7dd3fc",
    selectionBackground: "#334155",
  },
});
const fit = new FitAddon();
terminal.loadAddon(fit);

let sessionId: number | null = null;
let bytesReceived = 0;
let bytesParsed = 0;
let lastSize = { cols: 0, rows: 0 };
let inputQueue = Promise.resolve();
let resizeQueue = Promise.resolve();
let resizeTimes: number[] = [];
let payloadProbe: PayloadProbe | null = null;
let completedPayload: { probe: PayloadProbe; rawFinishedAt: number; parsedThrough: number } | null = null;
let pingProbe: PingProbe | null = null;
let busy = false;
let fiveTerminals: Array<{
  id: number | null;
  terminal: Terminal;
  received: number;
  parsed: number;
  channel: Channel<ArrayBuffer>;
  resizeObserver: ResizeObserver;
  inputQueue: Promise<void>;
  resizeQueue: Promise<void>;
}> = [];
let frameTracking = false;
let maxFrameGap = 0;
let previousFrame = 0;
let yesProbe: {
  carry: Uint8Array;
  active: boolean;
  stopping: boolean;
  startedAt: number;
  bytes: number;
  invalidBytes: number;
  phase: number;
  timer: number;
} | null = null;

let animationFrameId = 0;
let metricsTimer = 0;
let terminalResizeObserver: ResizeObserver | undefined;
let onWindowResize: (() => void) | undefined;

function setFivePane(index: number, element: Element | null) {
  fivePaneContents[index] = element as HTMLElement | null;
}

function setStatus(message: string) {
  status.value = message;
}

function text(value: string) {
  return new TextEncoder().encode(value);
}

function concat(left: Uint8Array, right: Uint8Array) {
  const joined = new Uint8Array(left.length + right.length);
  joined.set(left);
  joined.set(right, left.length);
  return joined;
}

function indexOf(haystack: Uint8Array, needle: Uint8Array) {
  outer: for (let start = 0; start <= haystack.length - needle.length; start++) {
    for (let offset = 0; offset < needle.length; offset++) {
      if (haystack[start + offset] !== needle[offset]) continue outer;
    }
    return start;
  }
  return -1;
}

function keepFrameGapMeasurement() {
  frameTracking = true;
  maxFrameGap = 0;
}

function animationFrame(time: number) {
  if (frameTracking && previousFrame > 0) {
    maxFrameGap = Math.max(maxFrameGap, time - previousFrame);
    resizeText.value = `${resizeSummary()} · max UI frame gap: ${maxFrameGap.toFixed(1)}ms`;
  }
  previousFrame = time;
  animationFrameId = requestAnimationFrame(animationFrame);
}

function resizeSummary() {
  if (!resizeTimes.length) return "Resize: —";
  const sorted = [...resizeTimes].sort((a, b) => a - b);
  const p95 = sorted[Math.ceil(sorted.length * 0.95) - 1] ?? sorted.at(-1)!;
  return `Resize n=${sorted.length}, median=${sorted[Math.floor(sorted.length / 2)].toFixed(2)}ms, p95=${p95.toFixed(2)}ms`;
}

function sendBytes(bytes: Uint8Array) {
  if (sessionId === null) return Promise.reject(new Error("No terminal is running"));
  const id = sessionId;
  const write = inputQueue.then(() => invoke<void>("terminal_write", { id, bytes: Array.from(bytes) }));
  inputQueue = write.catch(() => {});
  return write;
}

function sendText(value: string) {
  return sendBytes(text(value));
}

function consumePayload(data: Uint8Array) {
  const probe = payloadProbe;
  if (!probe) return;
  let pending = concat(probe.carry, data);
  if (!probe.started) {
    const beginAt = indexOf(pending, probe.begin);
    if (beginAt < 0) {
      probe.carry = pending.slice(Math.max(0, pending.length - probe.begin.length + 1));
      return;
    }
    pending = pending.slice(beginAt + probe.begin.length);
    probe.started = true;
    probe.startedAt = performance.now();
    keepFrameGapMeasurement();
  }

  const endAt = indexOf(pending, probe.end);
  const payloadEnd = endAt < 0 ? Math.max(0, pending.length - probe.end.length + 1) : endAt;
  const payloadBytes = pending.subarray(0, payloadEnd);
  probe.bytes += payloadBytes.length;
  for (const byte of payloadBytes) {
    if (byte !== 0x78) probe.invalidBytes++;
  }
  if (endAt >= 0) {
    payloadProbe = null;
    completedPayload = { probe, rawFinishedAt: performance.now(), parsedThrough: bytesReceived };
    finishPayloadWhenParsed();
  } else {
    probe.carry = pending.slice(payloadEnd);
  }
}

function finishPayloadWhenParsed() {
  if (!completedPayload || bytesParsed < completedPayload.parsedThrough) return;
  const { probe, rawFinishedAt, parsedThrough } = completedPayload;
  const rawElapsed = rawFinishedAt - probe.startedAt;
  const renderElapsed = performance.now() - probe.startedAt;
  const missing = Math.max(0, probe.expectedBytes - probe.bytes);
  const extra = Math.max(0, probe.bytes - probe.expectedBytes);
  completedPayload = null;
  busy = false;
  frameTracking = false;
  setStatus(
    `Payload: ${probe.bytes.toLocaleString()} bytes, missing=${missing}, extra=${extra}, corrupt=${probe.invalidBytes}, ` +
      `${(probe.bytes / rawElapsed / 1024 / 1024).toFixed(1)} MiB/s raw; xterm parsed ${parsedThrough.toLocaleString()} B ` +
      `in ${renderElapsed.toFixed(1)}ms · max UI frame gap ${maxFrameGap.toFixed(1)}ms`,
  );
  void compareChannelCounts();
}

function consumePing(data: Uint8Array) {
  const probe = pingProbe;
  if (!probe) return;
  const pending = concat(probe.carry, data);
  if (indexOf(pending, probe.marker) >= 0) {
    const elapsed = performance.now() - probe.startedAt;
    pingProbe = null;
    busy = false;
    setStatus(`PTY round-trip: ${elapsed.toFixed(2)}ms (Tauri write → PTY → Tauri Channel)`);
    return;
  }
  probe.carry = pending.slice(Math.max(0, pending.length - probe.marker.length + 1));
}

function consumeYes(data: Uint8Array) {
  const probe = yesProbe;
  if (!probe) return;
  let pending = concat(probe.carry, data);
  if (!probe.active) {
    const startAt = indexOf(pending, yesStart);
    if (startAt < 0) {
      probe.carry = pending.slice(Math.max(0, pending.length - yesStart.length + 1));
      return;
    }
    pending = pending.slice(startAt + yesStart.length);
    probe.active = true;
    probe.startedAt = performance.now();
    probe.bytes = yesLine.length;
    probe.phase = 0;
    keepFrameGapMeasurement();
    probe.timer = window.setTimeout(() => stopYes(), 30_000);
  }

  for (const byte of pending) {
    const expected = yesLine[probe.phase];
    if (probe.stopping && byte === 94) {
      finishYes();
      return;
    }
    if (byte !== expected) {
      probe.invalidBytes++;
      if (!probe.stopping) probe.phase = 0;
      continue;
    }
    probe.bytes++;
    probe.phase = (probe.phase + 1) % yesLine.length;
  }
  probe.carry = new Uint8Array();
}

function finishYes() {
  if (!yesProbe) return;
  const probe = yesProbe;
  clearTimeout(probe.timer);
  const elapsed = performance.now() - probe.startedAt;
  yesProbe = null;
  busy = false;
  frameTracking = false;
  setStatus(
    `yes: ${(elapsed / 1000).toFixed(2)}s, ${probe.bytes.toLocaleString()} bytes, ` +
      `${(probe.bytes / elapsed / 1024 / 1024).toFixed(1)} MiB/s, invalid=${probe.invalidBytes}, ` +
      `max UI frame gap=${maxFrameGap.toFixed(1)}ms`,
  );
  void compareChannelCounts();
}

function stopYes() {
  if (!yesProbe || yesProbe.stopping) return;
  yesProbe.stopping = true;
  void sendBytes(Uint8Array.of(3));
  window.setTimeout(() => {
    if (yesProbe) finishYes();
  }, 1500);
}

function acceptOutput(buffer: ArrayBuffer) {
  const data = new Uint8Array(buffer);
  bytesReceived += data.length;
  consumePayload(data);
  consumePing(data);
  consumeYes(data);
  terminal.write(data, () => {
    bytesParsed += data.length;
    finishPayloadWhenParsed();
  });
}

async function compareChannelCounts() {
  if (sessionId === null) return;
  let metrics: BackendMetrics | undefined;
  for (let attempt = 0; attempt < 20; attempt++) {
    try {
      metrics = await invoke<BackendMetrics>("terminal_metrics", { id: sessionId });
      if (metrics.bytesSent === bytesReceived) break;
    } catch {
      return;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 50));
  }
  if (!metrics) return;
  const difference = metrics.bytesSent - bytesReceived;
  setStatus(
    `${status.value} · channel byte delta=${difference} · backend dropped=${metrics.droppedBytes}`,
  );
}

async function openFiveTerminals() {
  if (sessionId !== null || fiveTerminals.length || fiveOpening.value || busy) return;
  fiveOpening.value = true;
  fiveMode.value = true;
  await nextTick();
  keepFrameGapMeasurement();
  resizeText.value = "Five PTYs · measuring max UI frame gap…";
  setStatus("Opening five independent PTYs and WebView channels…");
  try {
    for (let index = 0; index < 5; index++) {
      const content = fivePaneContents[index];
      if (!content) throw new Error(`Missing terminal pane ${index + 1}`);

      const view = new Terminal({
        cursorBlink: true,
        fontFamily: "SFMono-Regular, Menlo, Monaco, monospace",
        fontSize: 11,
        scrollback: 1000,
        theme: terminal.options.theme,
      });
      const fitAddon = new FitAddon();
      view.loadAddon(fitAddon);
      view.open(content);
      fitAddon.fit();
      const entry = {
        id: null as number | null,
        terminal: view,
        received: 0,
        parsed: 0,
        channel: new Channel<ArrayBuffer>(),
        resizeObserver: new ResizeObserver(() => fitAddon.fit()),
        inputQueue: Promise.resolve(),
        resizeQueue: Promise.resolve(),
      };
      entry.channel.onmessage = (buffer) => {
        const bytes = new Uint8Array(buffer);
        entry.received += bytes.length;
        view.write(bytes, () => {
          entry.parsed += bytes.length;
        });
      };
      view.onData((value) => {
        if (entry.id === null) return;
        const id = entry.id;
        const write = entry.inputQueue.then(() =>
          invoke<void>("terminal_write", { id, bytes: Array.from(text(value)) }),
        );
        entry.inputQueue = write.catch((error: unknown) => setStatus(`PTY input failed: ${String(error)}`));
      });
      view.onResize(({ cols, rows }) => {
        if (entry.id === null) return;
        const id = entry.id;
        const resize = entry.resizeQueue.then(() => invoke<void>("terminal_resize", { id, cols, rows }));
        entry.resizeQueue = resize.catch((error: unknown) => setStatus(`PTY resize failed: ${String(error)}`));
      });
      entry.resizeObserver.observe(content);

      fiveTerminals.push(entry);
      entry.id = await invoke<number>("terminal_start", {
        cwd: cwd.value,
        cols: view.cols,
        rows: view.rows,
        onOutput: entry.channel,
      });
      await invoke("terminal_write", {
        id: entry.id,
        bytes: Array.from(text(`printf 'MARVIS_FIVE_${index + 1}\\n'\r`)),
      });
    }
    setStatus("Five PTYs are live; each pane has its own Rust process, Channel<ArrayBuffer>, and xterm renderer.");
  } catch (error) {
    setStatus(`Five-terminal test failed: ${String(error)}`);
    await closeFiveTerminals();
  } finally {
    fiveOpening.value = false;
  }
}

async function closeFiveTerminals() {
  const active = fiveTerminals;
  fiveTerminals = [];
  for (const entry of active) {
    const id = entry.id;
    entry.id = null;
    entry.channel.onmessage = () => {};
    entry.resizeObserver.disconnect();
    entry.terminal.dispose();
    await entry.inputQueue;
    await entry.resizeQueue;
    if (id !== null) {
      try {
        await invoke("terminal_close", { id });
      } catch {
        // The PTY may already have exited.
      }
    }
  }
  fiveMode.value = false;
  frameTracking = false;
  resizeText.value = `${resizeSummary()} · five-terminal max UI frame gap: ${maxFrameGap.toFixed(1)}ms`;
  requestAnimationFrame(() => fit.fit());
}

terminal.onData((value) => {
  if (sessionId === null || busy) return;
  void sendText(value).catch((error: unknown) => setStatus(`PTY input failed: ${String(error)}`));
});

terminal.onResize(({ cols, rows }) => {
  if (sessionId === null || (cols === lastSize.cols && rows === lastSize.rows)) return;
  lastSize = { cols, rows };
  const started = performance.now();
  const id = sessionId;
  const resize = resizeQueue.then(() => invoke<[number, number]>("terminal_resize", { id, cols, rows }));
  resizeQueue = resize.then(
    () => undefined,
    (error: unknown) => {
      setStatus(`PTY resize failed: ${String(error)}`);
    },
  );
  void resize
    .then(() => {
      resizeTimes.push(performance.now() - started);
      resizeText.value = resizeSummary() + (frameTracking ? ` · max UI frame gap: ${maxFrameGap.toFixed(1)}ms` : "");
    })
    .catch(() => {});
});

async function startShell() {
  if (sessionId !== null || startingShell.value) return;
  startingShell.value = true;
  setStatus("Starting portable-pty shell…");
  try {
    fit.fit();
    bytesReceived = 0;
    bytesParsed = 0;
    inputQueue = Promise.resolve();
    resizeQueue = Promise.resolve();
    resizeTimes = [];
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => acceptOutput(buffer);
    sessionId = await invoke<number>("terminal_start", {
      cwd: cwd.value,
      cols: terminal.cols,
      rows: terminal.rows,
      onOutput: channel,
    });
    lastSize = { cols: terminal.cols, rows: terminal.rows };
    shellRunning.value = true;
    setStatus(`PTY ${sessionId} started · ${terminal.cols}×${terminal.rows}`);
  } catch (error) {
    setStatus(`Could not start PTY: ${String(error)}`);
  } finally {
    startingShell.value = false;
  }
}

async function closeShell() {
  if (sessionId === null) return;
  const id = sessionId;
  sessionId = null;
  payloadProbe = null;
  completedPayload = null;
  pingProbe = null;
  if (yesProbe) clearTimeout(yesProbe.timer);
  yesProbe = null;
  busy = false;
  await inputQueue;
  await resizeQueue;
  try {
    await invoke("terminal_close", { id });
    terminal.reset();
    setStatus(`PTY ${id} closed`);
  } catch (error) {
    setStatus(`PTY close failed: ${String(error)}`);
  }
  shellRunning.value = false;
}

async function measureLatency() {
  if (sessionId === null || busy) return;
  busy = true;
  const marker = text(`MARVIS_PING_${crypto.randomUUID().replaceAll("-", "")}`);
  pingProbe = { marker, carry: new Uint8Array(), startedAt: performance.now() };
  const command = `printf '%s' '${new TextDecoder().decode(marker)}'\r`;
  try {
    await sendText(command);
  } catch (error) {
    pingProbe = null;
    busy = false;
    setStatus(`PTY round-trip failed: ${String(error)}`);
  }
}

async function measurePayload() {
  if (sessionId === null || busy) return;
  const mebibytes = Math.max(1, Math.min(128, Number(payloadSize.value) || 1));
  payloadSize.value = String(mebibytes);
  const token = crypto.randomUUID().replaceAll("-", "");
  const begin = text(`\u001eMARVIS_BEGIN_${token}\u001f`);
  const end = text(`\u001eMARVIS_END_${token}\u001f`);
  const command = `${exactPayloadCommand(begin, end, mebibytes * 1024 * 1024)}\r`;
  busy = true;
  payloadProbe = {
    begin,
    end,
    carry: new Uint8Array(),
    started: false,
    startedAt: 0,
    bytes: 0,
    expectedBytes: mebibytes * 1024 * 1024,
    invalidBytes: 0,
  };
  try {
    await sendText(command);
  } catch (error) {
    payloadProbe = null;
    busy = false;
    setStatus(`Payload test failed: ${String(error)}`);
  }
}

async function startYes() {
  if (sessionId === null || busy) return;
  busy = true;
  yesProbe = {
    carry: new Uint8Array(),
    active: false,
    stopping: false,
    startedAt: 0,
    bytes: 0,
    invalidBytes: 0,
    phase: 0,
    timer: 0,
  };
  setStatus("Waiting for yes output; the 30s timer starts on its first line.");
  try {
    await sendText("yes\r");
  } catch (error) {
    yesProbe = null;
    busy = false;
    setStatus(`yes failed: ${String(error)}`);
  }
}

onMounted(() => {
  terminal.open(terminalElement.value!);
  fit.fit();
  terminalResizeObserver = new ResizeObserver(() => fit.fit());
  terminalResizeObserver.observe(terminalElement.value!);
  onWindowResize = () => fit.fit();
  window.addEventListener("resize", onWindowResize);
  animationFrameId = requestAnimationFrame(animationFrame);

  void invoke<string>("default_cwd")
  .then((defaultCwd) => {
    cwd.value = defaultCwd;
    setStatus("Renderer ready · start a shell to begin");
  })
  .catch((error: unknown) => setStatus(`Could not read initial directory: ${String(error)}`));

metricsTimer = window.setInterval(async () => {
  if (fiveTerminals.length) {
    const entries = fiveTerminals.filter((entry): entry is typeof entry & { id: number } => entry.id !== null);
    try {
      const snapshots = await Promise.all(
        entries.map((entry) => invoke<BackendMetrics>("terminal_metrics", { id: entry.id })),
      );
      const received = entries.reduce((total, entry) => total + entry.received, 0);
      const parsed = entries.reduce((total, entry) => total + entry.parsed, 0);
      const sent = snapshots.reduce((total, metrics) => total + metrics.bytesSent, 0);
      const dropped = snapshots.reduce((total, metrics) => total + metrics.droppedBytes, 0);
      metricsText.value =
        `Five PTYs · received ${received.toLocaleString()} B · parsed ${parsed.toLocaleString()} B · ` +
        `backend sent ${sent.toLocaleString()} B · delta ${sent - received} B · dropped ${dropped} B`;
    } catch {
      // The five-session test may be closing.
    }
    return;
  }
  if (sessionId === null) return;
  try {
    const metrics = await invoke<BackendMetrics>("terminal_metrics", { id: sessionId });
    const channelDelta = metrics.bytesSent - bytesReceived;
    const rate = bytesReceived / 1024 / 1024;
    metricsText.value =
      `Channel in ${rate.toFixed(2)} MiB · renderer parsed ${(bytesParsed / 1024 / 1024).toFixed(2)} MiB · ` +
      `backend read/sent ${metrics.bytesRead.toLocaleString()}/${metrics.bytesSent.toLocaleString()} B · ` +
      `channel delta ${channelDelta.toLocaleString()} B · backend dropped ${metrics.droppedBytes} B`;
  } catch {
    // The session may be closing.
  }
}, 750);
});

onUnmounted(() => {
  cancelAnimationFrame(animationFrameId);
  window.clearInterval(metricsTimer);
  if (onWindowResize) window.removeEventListener("resize", onWindowResize);
  terminalResizeObserver?.disconnect();
  for (const entry of fiveTerminals) {
    entry.resizeObserver.disconnect();
    entry.terminal.dispose();
  }
  terminal.dispose();
});
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <main class="mx-auto grid h-screen min-h-[480px] w-full max-w-[1440px] grid-rows-[auto_auto_minmax(180px,1fr)_auto] gap-4 px-7 py-6 max-[900px]:px-[18px] max-[900px]:py-[18px]">
      <header class="flex items-end justify-between gap-6 max-[900px]:flex-col max-[900px]:items-stretch">
        <div>
          <p class="mb-1.5 font-mono text-[11px] font-semibold uppercase tracking-[0.12em] text-sky-300">0.2 · terminal pipeline spike</p>
          <h1 class="m-0 text-[clamp(19px,2.1vw,26px)] font-semibold tracking-tight">portable-pty → Tauri Channel → xterm.js</h1>
        </div>
        <div class="grid min-w-[min(57%,660px)] grid-cols-[minmax(180px,1fr)_auto_auto] gap-2 max-[900px]:min-w-0">
          <label for="cwd" class="col-span-full text-[11px] text-slate-400">Working directory</label>
          <Input id="cwd" v-model="cwd" autocomplete="off" spellcheck="false" class="font-mono" aria-label="Working directory" />
          <Button :disabled="startingShell || shellRunning || fiveMode" @click="startShell">Start shell</Button>
          <Button variant="outline" :disabled="!shellRunning" @click="closeShell">Close shell</Button>
          <Button :disabled="shellRunning || fiveMode" @click="openFiveTerminals">Start five PTYs</Button>
          <Button variant="outline" :disabled="!fiveMode || fiveOpening" @click="closeFiveTerminals">Close five PTYs</Button>
        </div>
      </header>

      <section class="flex min-h-[39px] flex-wrap items-center gap-2" aria-label="Terminal measurements">
        <Button variant="outline" :disabled="!shellRunning || busy" @click="measureLatency">Measure PTY round-trip</Button>
        <label for="payload-size" class="inline-flex items-center gap-2 text-xs text-slate-400">Payload (MiB)</label>
        <Input id="payload-size" v-model="payloadSize" type="number" min="1" max="128" class="w-[70px]" aria-label="Payload size in mebibytes" />
        <TooltipRoot>
          <TooltipTrigger as-child>
            <button type="button" class="inline-flex size-6 items-center justify-center rounded-full border border-slate-700 text-xs text-slate-400 hover:text-sky-300" aria-label="About exact payload measurement">i</button>
          </TooltipTrigger>
          <TooltipContent class="z-50 max-w-64 rounded-md border border-slate-700 bg-slate-900 px-3 py-2 text-xs text-slate-200 shadow-xl" side="top">
            A Node.js process writes an exact count of x bytes between unique markers.
          </TooltipContent>
        </TooltipRoot>
        <Button variant="outline" :disabled="!shellRunning || busy" @click="measurePayload">Measure exact payload</Button>
        <Button variant="outline" :disabled="!shellRunning || busy" @click="startYes">Run yes · 30s</Button>
        <span class="ml-auto min-w-[220px] text-right font-mono text-[11px] leading-relaxed text-slate-400 max-[900px]:ml-0 max-[900px]:min-w-0 max-[900px]:text-left" role="status">{{ status }}</span>
      </section>

      <section ref="terminalElement" v-show="!fiveMode" class="min-h-0 overflow-hidden rounded-[10px] border border-slate-700/70 bg-[#10151d] p-2.5 shadow-2xl" aria-label="Terminal" />
      <section v-show="fiveMode" class="grid min-h-0 grid-cols-2 grid-rows-3 gap-2 overflow-hidden rounded-[10px] border border-slate-700/70 bg-[#10151d] p-2" aria-label="Five simultaneous terminals">
        <article v-for="index in 5" :key="index" class="multi-terminal-pane">
          <header class="h-[17px] font-mono text-[10px] leading-[17px] text-sky-300">PTY {{ index }}</header>
          <div :ref="(element) => setFivePane(index - 1, element as Element | null)" class="multi-terminal-content" />
        </article>
      </section>

      <footer class="grid gap-1 font-mono text-[11px] leading-relaxed text-slate-400">
        <div>{{ metricsText }}</div>
        <div>{{ resizeText }}</div>
        <p class="m-0 font-sans text-[11px] leading-relaxed text-slate-500">Headless PTY checks cannot measure this WebView. Keep this window visible during the 30s run.</p>
      </footer>
    </main>
  </TooltipProvider>
</template>
