<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from "vue";
import type { DocumentMode, MainView } from "../domain/main-document";
import type { EditorSettings, TerminalSettings } from "../domain/settings";
import type { Checkout, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";
import { DEFAULT_SETTINGS } from "../domain/settings";
import DocumentPane from "./DocumentPane.vue";
import SessionPane from "./SessionPane.vue";

// The diff renderer and its library stay out of the first chunk: the panel opens on a terminal.
const FileDiff = defineAsyncComponent(() => import("./FileDiff.vue"));

type ReviewApi = Pick<
  ActiveReviewNotes,
  "notes" | "addNote" | "updateNote" | "deleteNote" | "verifyAnchors" | "clearOutdated" | "resolveNote"
>;

const props = withDefaults(
  defineProps<{
    checkout: Checkout | null;
    /** Every checkout on the panel, which is what a terminal can be moved to. */
    checkouts?: Checkout[];
    view: MainView;
    ready: boolean;
    gitSnapshot: ActiveGitSnapshot;
    review: ReviewApi;
    activeSessionId: string | null;
    isOpening: boolean;
    shellRequest?: { checkoutId: string; token: number } | null;
    registeredSessionIds?: string[];
    refreshRevision?: number;
    readingPosition?: { top: number; left: number };
    diffScrollTop?: number;
    split?: boolean;
    previewWidth?: number;
    terminalSettings?: TerminalSettings;
    editorSettings?: EditorSettings;
    zoom?: number;
  }>(),
  {
    refreshRevision: 0,
    readingPosition: () => ({ top: 0, left: 0 }),
    checkouts: () => [],
    diffScrollTop: 0,
    shellRequest: null,
    registeredSessionIds: () => [],
    split: false,
    previewWidth: 360,
    terminalSettings: () => DEFAULT_SETTINGS.terminal,
    editorSettings: () => DEFAULT_SETTINGS.editor,
    zoom: 1,
  },
);
const emit = defineEmits<{
  openFolder: [];
  workspaceUpdated: [workspace: WorkspaceState];
  sessionStatusChanged: [sessionId: string, status: TerminalSessionStatus | null];
  updateDocumentMode: [mode: DocumentMode];
  readingPositionChanged: [position: { top: number; left: number }];
  diffPositionChanged: [top: number];
  openMarkdownLink: [path: string];
  openFile: [path: string];
  resizePreview: [width: number];
  closePreview: [];
}>();

const sessionPane = ref<InstanceType<typeof SessionPane> | null>(null);
const mainViewRoot = ref<HTMLElement | null>(null);
let resizeRootRight = 0;
let resizePointerId: number | null = null;
defineExpose({
  focusActiveTerminal: () => sessionPane.value?.focusActiveTerminal(),
  requestClose: (sessionId: string) => sessionPane.value?.requestClose(sessionId) ?? Promise.resolve(false),
  moveSession: (sessionId: string, targetCheckoutId: string, index: number) =>
    sessionPane.value?.moveSession(sessionId, targetCheckoutId, index) ?? Promise.resolve(),
});

/** The three views, one of them visible (D.1). */
const terminal = computed(() => props.view.kind === "terminal");
const documentView = computed(() => (props.view.kind === "document" ? props.view : null));
const diffView = computed(() => (props.view.kind === "diff" ? props.view : null));

function startPreviewResize(event: PointerEvent) {
  resizePointerId = event.pointerId;
  resizeRootRight = mainViewRoot.value?.getBoundingClientRect().right ?? event.clientX;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}

function resizePreview(event: PointerEvent) {
  if (!props.split || resizePointerId !== event.pointerId) return;
  // The pointer and the rect are both in the window's units, so the drag distance is honest; what
  // the preview is sized in is the app's, and the two differ by whatever the window is scaled to.
  emit("resizePreview", (resizeRootRight - event.clientX) / props.zoom - 5);
}

function finishPreviewResize(event: PointerEvent) {
  if (resizePointerId !== event.pointerId) return;
  resizePointerId = null;
  const handle = event.currentTarget as HTMLElement;
  if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
}

function onPreviewResizeKeydown(event: KeyboardEvent) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  emit("resizePreview", props.previewWidth + (event.key === "ArrowLeft" ? 20 : -20));
}
</script>

<template>
  <div ref="mainViewRoot" class="relative min-h-0 flex-1 overflow-hidden" :class="{ flex: split }" :aria-busy="!ready">
    <!-- The terminal section is never unmounted: xterm mis-measures from a hidden box, and
         `visible` is what tells it to hold off until the panel is really on screen. -->
    <section
      v-show="split || terminal"
      id="main-view-terminal"
      :class="split ? 'relative min-h-0 min-w-[320px] flex-1' : 'absolute inset-0'"
    >
      <SessionPane
        ref="sessionPane"
        class="absolute inset-0"
        :checkout="ready ? checkout : null"
        :checkouts="checkouts"
        :active-session-id="activeSessionId"
        :is-opening="isOpening || !ready"
        :visible="ready && (split || terminal)"
        :shell-request="shellRequest"
        :registered-session-ids="registeredSessionIds"
        :terminal-settings="terminalSettings"
        :zoom="zoom"
        @open-folder="$emit('openFolder')"
        @workspace-updated="$emit('workspaceUpdated', $event)"
        @session-status-changed="(sessionId, status) => $emit('sessionStatusChanged', sessionId, status)"
        @open-file="$emit('openFile', $event)"
      />
    </section>
    <div
      v-if="split && !terminal"
      role="separator"
      aria-label="Resize preview panel"
      aria-orientation="vertical"
      :aria-valuemin="260"
      :aria-valuemax="900"
      :aria-valuenow="previewWidth"
      tabindex="0"
      class="splitter-handle shrink-0"
      @pointerdown.prevent="startPreviewResize"
      @pointermove="resizePreview"
      @pointerup="finishPreviewResize"
      @pointercancel="finishPreviewResize"
      @keydown="onPreviewResizeKeydown"
    >
      <div
        aria-hidden="true"
        class="absolute left-1/2 top-1/2 h-6 w-1 -translate-x-1/2 -translate-y-1/2 bg-(--marvis-text-faint)"
      />
    </div>
    <section
      v-show="!terminal"
      id="main-view-preview"
      :class="split ? 'relative min-h-0 min-w-[260px] max-w-[900px] shrink-0' : 'absolute inset-0'"
      :style="split ? { width: `min(${previewWidth}px, calc(100% - 325px))` } : undefined"
    >
      <div class="relative h-full min-h-0 w-full">
        <section v-show="documentView" id="main-view-document" class="absolute inset-0">
          <DocumentPane
            class="absolute inset-0"
            :checkout="checkout"
            :path="documentView?.path ?? null"
            :origin="documentView?.origin ?? 'checkout'"
            :mode="documentView?.mode ?? 'code'"
            :git-snapshot="gitSnapshot"
            :refresh-revision="refreshRevision"
            :reading-position="readingPosition"
            :editor-settings="editorSettings"
            @update-mode="$emit('updateDocumentMode', $event)"
            @reading-position-changed="$emit('readingPositionChanged', $event)"
            @open-markdown-link="$emit('openMarkdownLink', $event)"
            @close="$emit('closePreview')"
          />
        </section>
        <section v-show="diffView" id="main-view-diff" class="absolute inset-0">
          <!-- A height, not a position: the diff is its own positioned box (the composer of a large
               diff is a layer over its bottom edge), and Tailwind emits `.absolute` before
               `.relative`, so a panel that also said `absolute` here would be overruled by that and
               `inset-0` would stop giving the diff a height. With no height the diff's own
               scroll container has nothing to scroll in and the file is drawn at its full length
               under a panel that clips it. -->
          <FileDiff
            v-if="diffView && checkout"
            class="h-full"
            :checkout="checkout"
            :git-snapshot="gitSnapshot"
            :review="review"
            :path="diffView.path"
            :scroll-top="diffScrollTop"
            :editor-settings="editorSettings"
            @scroll-position-changed="$emit('diffPositionChanged', $event)"
            @close="$emit('closePreview')"
          />
        </section>
      </div>
    </section>
  </div>
</template>
