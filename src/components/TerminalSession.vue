<script setup lang="ts">
import { Channel } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { TerminalSessionStatus } from "../domain/workspace";
import type { TerminalScrollbarMode } from "../domain/settings";
import { closeTerminal, createTerminal, getTerminalStatus, resizeTerminal, writeTerminal } from "../lib/ipc";
import {
  attachTerminalRenderer,
  createMarvisTerminal,
  setTerminalLigatures,
  enableTerminalSelectionCopy,
  preloadTerminalFonts,
  terminalFontSize,
} from "../lib/marvis-terminal";
import { renderPtyOutput } from "../lib/terminal-renderer";
import { scrollbarOffsetForTop, terminalScrollbarGeometry } from "../lib/terminal-scrollbar";
import type { TerminalScrollbarGeometry } from "../lib/terminal-scrollbar";
import { useToasts } from "../presentation/toasts";

const props = withDefaults(
  defineProps<{
    checkoutId: string;
    active: boolean;
    visible?: boolean;
    focused?: boolean;
    scrollbar?: TerminalScrollbarMode;
    fontSize?: number;
    ligatures?: boolean;
    cursorBlink?: boolean;
    zoom?: number;
  }>(),
  { visible: true, focused: false, scrollbar: "hidden", fontSize: 16, ligatures: true, cursorBlink: true, zoom: 1 },
);
const emit = defineEmits<{
  created: [result: Awaited<ReturnType<typeof createTerminal>>];
  closed: [workspace: Awaited<ReturnType<typeof closeTerminal>>];
  statusChanged: [status: TerminalSessionStatus];
  failed: [message: string];
}>();

const terminalElement = ref<HTMLElement | null>(null);
const scrollbarTrack = ref<HTMLElement | null>(null);
const scrollbarThumb = ref({ top: 0, height: 0 });
const scrollbarLit = ref(false);
const scrollbarPosition = ref(0);
const scrollbarMaximum = ref(0);
const state = ref<TerminalSessionStatus>({ state: "running", foregroundProcess: false });
const error = ref<string | null>(null);
const closing = ref(false);
const terminal = createMarvisTerminal(props.fontSize, props.cursorBlink, props.zoom);
const fit = new FitAddon();
terminal.loadAddon(fit);
const { pushCause } = useToasts();

/** The scrollbar is Marvis' own and only exists in the two modes that draw one. */
const scrollbarEnabled = computed(() => props.scrollbar !== "hidden");
/** `always` has no fade to get to, so its thumb is simply there. */
const scrollbarVisible = computed(() => props.scrollbar === "always" || scrollbarLit.value);

let sessionId: string | null = null;
let channel: Channel<ArrayBuffer> | undefined;
let resizeObserver: ResizeObserver | undefined;
let statusTimer: number | undefined;
let inputQueue: Promise<void> = Promise.resolve();
let resizeQueue: Promise<void> = Promise.resolve();
let resizeScheduled = false;
let disposed = false;
let started = false;
let terminalReady = false;
let selectionCopy: { dispose(): void } | undefined;
let latestSize = { cols: 0, rows: 0 };
let geometry: TerminalScrollbarGeometry = { top: 0, height: 0, scrollable: false, maxOffset: 0, travel: 0 };
let scrollbarFadeTimer: number | undefined;
let scrollbarDrag: { pointerId: number; grabOffset: number } | null = null;
let wheelRemainderPx = 0;

/**
 * How long `auto` leaves the thumb up after the last scroll.
 *
 * Long enough to read where the thumb landed and to catch it again, short enough that a terminal
 * nobody is scrolling looks like a terminal with no scrollbar — which is what the setting is for
 * people who scrolled once and decided they did not need one.
 */
const SCROLLBAR_FADE_MS = 900;

function showError(cause: unknown) {
  error.value = cause instanceof Error ? cause.message : String(cause);
}

/**
 * Reads the scrollback and puts the thumb where it belongs.
 *
 * The track's own height is measured rather than assumed because the pane is whatever height the
 * splitter and the layout leave it, and the arithmetic is in `terminal-scrollbar.ts` over four
 * numbers. Writing the style only when the numbers moved is what keeps this cheap enough to call
 * on every scroll: a terminal that prints for a minute fires this on every line, and a scrollback
 * whose length is not a multiple of anything will otherwise re-render the pane at the same size
 * a thousand times.
 */
function updateScrollbar() {
  if (!scrollbarEnabled.value) return;
  const scrollback = terminal.buffer.active;
  geometry = terminalScrollbarGeometry(
    { length: scrollback.length, viewportY: scrollback.viewportY, rows: terminal.rows },
    scrollbarTrack.value?.clientHeight ?? 0,
  );
  scrollbarPosition.value = Math.min(geometry.maxOffset, Math.max(0, scrollback.viewportY));
  scrollbarMaximum.value = geometry.maxOffset;
  // A terminal whose whole history fits on screen has nothing to say with a thumb, so in `auto` it
  // does not get to flash one. `always` still draws it: a full-height thumb is an honest answer,
  // and a person who asked for a permanent scrollbar is entitled to see that there is nothing to
  // scroll rather than to wonder whether the setting took.
  if (!geometry.scrollable) scrollbarLit.value = false;
  if (geometry.top === scrollbarThumb.value.top && geometry.height === scrollbarThumb.value.height) return;
  scrollbarThumb.value = { top: geometry.top, height: geometry.height };
}

/**
 * `auto` shows the thumb while something is moving and takes it away when nothing is.
 *
 * The timer restarts on every scroll instead of running once, so a build that prints for a minute
 * keeps its scrollbar up for that minute — the thumb genuinely is moving the whole time — and the
 * fade happens when the output stops. Dragging counts as scrolling, and so does the wheel over
 * the track, or the thumb would fade out from under a pointer that is still holding it.
 */
function wakeScrollbar() {
  if (!scrollbarEnabled.value || props.scrollbar === "always" || !geometry.scrollable) return;
  scrollbarLit.value = true;
  if (scrollbarFadeTimer !== undefined) window.clearTimeout(scrollbarFadeTimer);
  scrollbarFadeTimer = undefined;
  if (scrollbarDrag) return;
  scrollbarFadeTimer = window.setTimeout(() => {
    scrollbarFadeTimer = undefined;
    scrollbarLit.value = false;
  }, SCROLLBAR_FADE_MS);
}

/** Moves the viewport to where a thumb dropped at `top` would stand, and re-reads it. */
function scrollbarTo(top: number) {
  terminal.scrollToLine(scrollbarOffsetForTop(top, geometry));
  updateScrollbar();
}

function onScrollbarPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  const track = scrollbarTrack.value;
  if (!track) return;
  updateScrollbar();
  if (!geometry.scrollable) return;
  // The track is inside the scaled window and the pointer is not, so the offset is divided back
  // into the track's own units: the geometry it is compared against is measured in those.
  const y = (event.clientY - track.getBoundingClientRect().top) / props.zoom;
  // A press on the thumb takes hold of it wherever it was caught, so the thumb does not jump to
  // centre itself under the pointer. A press on the empty track puts the middle of the thumb there
  // instead, which is what every other scrollbar does and is what makes clicking above or below
  // the thumb a jump rather than a press that does nothing at all.
  const grabOffset = y >= geometry.top && y <= geometry.top + geometry.height ? y - geometry.top : geometry.height / 2;
  scrollbarDrag = { pointerId: event.pointerId, grabOffset };
  track.setPointerCapture(event.pointerId);
  scrollbarTo(y - grabOffset);
  wakeScrollbar();
  event.preventDefault();
}

function onScrollbarPointerMove(event: PointerEvent) {
  const track = scrollbarTrack.value;
  if (!track || !scrollbarDrag || scrollbarDrag.pointerId !== event.pointerId) return;
  scrollbarTo((event.clientY - track.getBoundingClientRect().top) / props.zoom - scrollbarDrag.grabOffset);
  wakeScrollbar();
  event.preventDefault();
}

function onScrollbarPointerUp(event: PointerEvent) {
  if (!scrollbarDrag || scrollbarDrag.pointerId !== event.pointerId) return;
  const pointerId = scrollbarDrag.pointerId;
  scrollbarDrag = null;
  // The capture is released rather than left to the browser, so a view that is torn down mid-drag
  // does not leave this holding a pointer id that belongs to a pane that is no longer there.
  if (scrollbarTrack.value?.hasPointerCapture(pointerId)) {
    scrollbarTrack.value.releasePointerCapture(pointerId);
  }
  wakeScrollbar();
}

function onScrollbarLostPointerCapture(event: PointerEvent) {
  if (!scrollbarDrag || scrollbarDrag.pointerId !== event.pointerId) return;
  scrollbarDrag = null;
  wakeScrollbar();
}

function cancelScrollbarDrag() {
  const pointerId = scrollbarDrag?.pointerId;
  scrollbarDrag = null;
  if (pointerId !== undefined && scrollbarTrack.value?.hasPointerCapture(pointerId)) {
    scrollbarTrack.value.releasePointerCapture(pointerId);
  }
}

/**
 * The overlay is a sibling of the terminal rather than part of it, so a wheel over it never
 * reaches xterm's own viewport. Preserve the event's line or pixel delta instead of turning a
 * small trackpad gesture into a screenful, and accumulate pixel deltas until they cover a row.
 */
function onScrollbarWheel(event: WheelEvent) {
  if (!geometry.scrollable || event.deltaY === 0) return;
  event.preventDefault();
  const cellHeight = (scrollbarTrack.value?.clientHeight ?? 0) / terminal.rows;
  if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
    wheelRemainderPx = 0;
    terminal.scrollLines(Math.trunc(event.deltaY) * terminal.rows);
    return;
  }
  const deltaPx = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? event.deltaY * cellHeight : event.deltaY;
  wheelRemainderPx += deltaPx;
  const lines = Math.trunc(wheelRemainderPx / Math.max(1, cellHeight));
  if (lines === 0) return;
  wheelRemainderPx -= lines * cellHeight;
  terminal.scrollLines(lines);
  wakeScrollbar();
}

function onScrollbarKeydown(event: KeyboardEvent) {
  if (!geometry.scrollable) return;
  const page = Math.max(1, terminal.rows - 1);
  const next = (() => {
    switch (event.key) {
      case "ArrowUp":
        return scrollbarPosition.value - 1;
      case "ArrowDown":
        return scrollbarPosition.value + 1;
      case "PageUp":
        return scrollbarPosition.value - page;
      case "PageDown":
        return scrollbarPosition.value + page;
      case "Home":
        return 0;
      case "End":
        return geometry.maxOffset;
      default:
        return null;
    }
  })();
  if (next === null) return;
  event.preventDefault();
  terminal.scrollToLine(Math.min(geometry.maxOffset, Math.max(0, next)));
  updateScrollbar();
  wakeScrollbar();
}

function updateStatus(status: TerminalSessionStatus) {
  state.value = status;
  emit("statusChanged", status);
  if (status.state !== "exited") return;
  if (statusTimer !== undefined) {
    window.clearInterval(statusTimer);
    statusTimer = undefined;
  }
  // A shell that has exited has nothing left to say, and holding its last screen open is worse
  // than losing it: the panel shows a frozen frame that looks like a hung app, the session stays
  // in the sidebar with a live-looking entry, and the only way out is the close button on a pane
  // the user has to go looking for. `exit` and `:q` are how a shell is ended on purpose, so the
  // session goes with it. The close asks the backend once more before acting, finds the process
  // already gone, and so never reaches the confirmation that stopping a live process needs — that
  // question stays where it belongs, on the close the user asked for.
  //
  // This runs for any status report, including the one `requestClose` makes on its own way out,
  // and the `closing` guard there turns that second call into a no-op instead of a loop.
  void requestClose();
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
  if (!terminalReady || !props.active || !props.visible || !terminalElement.value) return;
  if (terminalElement.value.clientWidth === 0 || terminalElement.value.clientHeight === 0) return;
  fit.fit();
  // The thumb is a fraction of the pane, so it is measured where the pane is measured, and this is
  // where the pane changes size. It goes before the column recovery below rather than after it
  // because that recovery gives columns back and keeps the rows: a terminal that fits is the same
  // height as one that does not, so there is nothing about the thumb that changes after it.
  updateScrollbar();
  // FitAddon reserves 14px for an overview ruler whenever scrollback is enabled, even though
  // this terminal hides the ruler and scrollbar. Recover those columns without clipping TUIs.
  const screen = terminal.element?.querySelector(".xterm-screen");
  if (!screen) return;
  const cellWidth = screen.getBoundingClientRect().width / terminal.cols;
  if (!cellWidth) return;
  const cols = Math.floor(terminalElement.value.getBoundingClientRect().width / cellWidth);
  if (cols > terminal.cols) terminal.resize(cols, terminal.rows);
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
    // Recorded but not published: a process that ended between two polls has already reported its
    // own ending, and this read exists to be sure before stopping something, not to say it twice.
    state.value = actualStatus;
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
  if (started || disposed || !terminalReady || !props.active || !terminalElement.value) return;
  started = true;
  fitActiveView();
  channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buffer) => renderPtyOutput(terminal, buffer);
  const initialSize = { cols: terminal.cols || 80, rows: terminal.rows || 24 };
  try {
    const created = await createTerminal(props.checkoutId, initialSize.cols, initialSize.rows, channel);
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
// xterm fires this whenever the viewport moves, whether a wheel, a drag, Shift+PageUp or output
// arriving at the bottom caused it, and it does not promise what the payload is — some paths send
// the new position, some send an object wrapping it — so the position is read back off the buffer
// rather than taken from the event. Output is the reason this is worth subscribing to at all: a
// scrollback that grows has to shrink its thumb, and nothing else says so.
terminal.onScroll(() => {
  updateScrollbar();
  wakeScrollbar();
});

/**
 * Picking a mode is a change to what the pane draws, so the thumb is measured again once the
 * overlay is really there — the element only exists in the modes that draw one, and a thumb sized
 * against a track that was not in the document yet would be sized against nothing.
 */
watch(
  () => props.scrollbar,
  async (mode) => {
    if (scrollbarFadeTimer !== undefined) window.clearTimeout(scrollbarFadeTimer);
    scrollbarFadeTimer = undefined;
    if (mode === "hidden") {
      scrollbarLit.value = false;
      wheelRemainderPx = 0;
      cancelScrollbarDrag();
      return;
    }
    await nextTick();
    updateScrollbar();
    // Turning `auto` on shows the thumb once, so the answer to "did that do anything" is visible,
    // and from there the same fade as a scroll gets it out of the way again.
    wakeScrollbar();
  },
);

/**
 * The size and the two faces of the terminal's own type, watched together because they all end in
 * the same fit.
 *
 * xterm.js re-measures the cell, clears and repaints on the fontSize assignment, so the only thing
 * left is the fit: the PTY is told its new column count through it, and nobody is told anything by
 * the repaint. The ligature joiner and the cursor are options on the same object, and both take
 * effect on the next paint, so neither needs one.
 */
watch(
  () => [props.fontSize, props.ligatures, props.cursorBlink, props.zoom] as const,
  async ([fontSize, ligatures, cursorBlink, zoom]) => {
    if (!terminalReady) return;
    terminal.options.fontSize = terminalFontSize(fontSize, zoom);
    terminal.options.cursorBlink = cursorBlink;
    setTerminalLigatures(terminal, ligatures);
    await nextTick();
    fitActiveView();
  },
);

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

onMounted(async () => {
  if (!terminalElement.value) return;
  // Let xterm measure and rasterize only after both faces it can draw have loaded. Clearing a
  // WebGL atlas after the fallback was already painted made the first selected cells differ from
  // their neighbours; the bundled faces make waiting here local and deterministic. The load is
  // kicked off when the app starts rather than here, so on any terminal but the first this is
  // already settled and the await costs a microtask.
  await preloadTerminalFonts();
  if (disposed || !terminalElement.value) return;
  terminal.open(terminalElement.value);
  terminalReady = true;
  // Both of these need the terminal on the page, and the fit that follows has to measure the
  // renderer that will actually draw.
  setTerminalLigatures(terminal, props.ligatures);
  attachTerminalRenderer(terminal);
  selectionCopy = enableTerminalSelectionCopy(terminal, (text) => {
    void writeText(text).catch((cause) => {
      pushCause(cause);
    });
  });
  fitActiveView();
  resizeObserver = new ResizeObserver(() => fitActiveView());
  resizeObserver.observe(terminalElement.value);
  void startSession();
});

onUnmounted(() => {
  disposed = true;
  if (statusTimer !== undefined) window.clearInterval(statusTimer);
  if (scrollbarFadeTimer !== undefined) window.clearTimeout(scrollbarFadeTimer);
  scrollbarDrag = null;
  resizeObserver?.disconnect();
  selectionCopy?.dispose();
  if (channel) channel.onmessage = () => {};
  terminal.dispose();
});
</script>

<template>
  <section class="terminal-surface flex h-full min-h-0 flex-col overflow-hidden">
    <div class="terminal-region relative min-h-0 flex-1">
      <!-- The host cancels out the root's scale rather than inheriting it: the grid is measured
           and rasterized in the screen's own pixels, and the cell is drawn at the scaled size. -->
      <div
        ref="terminalElement"
        class="terminal-host h-full min-h-0"
        :style="{ zoom: String(1 / props.zoom) }"
        aria-label="Shell terminal"
      />
      <!-- Overlaying the terminal preserves the grid width that xterm's native bar would reserve. -->
      <div
        v-if="scrollbarEnabled"
        ref="scrollbarTrack"
        class="terminal-scrollbar"
        :class="{ 'terminal-scrollbar-idle': !scrollbarVisible }"
        role="scrollbar"
        aria-label="Terminal scrollback"
        aria-orientation="vertical"
        aria-valuemin="0"
        :aria-valuemax="scrollbarMaximum"
        :aria-valuenow="scrollbarPosition"
        tabindex="0"
        @keydown="onScrollbarKeydown"
        @pointerdown="onScrollbarPointerDown"
        @pointermove="onScrollbarPointerMove"
        @pointerup="onScrollbarPointerUp"
        @pointercancel="onScrollbarPointerUp"
        @lostpointercapture="onScrollbarLostPointerCapture"
        @wheel="onScrollbarWheel"
      >
        <div
          class="terminal-scrollbar-thumb"
          :style="{ top: `${scrollbarThumb.top}px`, height: `${scrollbarThumb.height}px` }"
        />
      </div>
    </div>
    <p v-if="error" role="alert" class="m-0 border-t border-(--marvis-border) px-3 py-2 text-xs text-(--marvis-red)">
      {{ error }}
    </p>
  </section>
</template>
