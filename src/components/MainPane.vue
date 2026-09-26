<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from "vue";
import type { DocumentMode, MainView } from "../domain/main-document";
import type { Checkout, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";
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
    view: MainView;
    ready: boolean;
    gitSnapshot: ActiveGitSnapshot;
    review: ReviewApi;
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
    zedAvailable?: boolean;
    neovimAvailable?: boolean;
    refreshRevision?: number;
    readingPosition?: { top: number; left: number };
    diffScrollTop?: number;
  }>(),
  {
    zedAvailable: false,
    neovimAvailable: false,
    refreshRevision: 0,
    readingPosition: () => ({ top: 0, left: 0 }),
    diffScrollTop: 0,
    shellRequest: null,
    nvimRequest: null,
    registeredSessionIds: () => [],
  },
);
defineEmits<{
  openFolder: [];
  workspaceUpdated: [workspace: WorkspaceState];
  sessionStatusChanged: [sessionId: string, status: TerminalSessionStatus | null];
  updateDocumentMode: [mode: DocumentMode];
  readingPositionChanged: [position: { top: number; left: number }];
  diffPositionChanged: [top: number];
  openMarkdownLink: [path: string];
  openInZed: [];
  openInNeovim: [];
}>();

const sessionPane = ref<InstanceType<typeof SessionPane> | null>(null);
defineExpose({
  focusActiveTerminal: () => sessionPane.value?.focusActiveTerminal(),
  requestClose: (sessionId: string) => sessionPane.value?.requestClose(sessionId) ?? Promise.resolve(false),
});

/** The three views, one of them visible (D.1). */
const terminal = computed(() => props.view.kind === "terminal");
const documentView = computed(() => (props.view.kind === "document" ? props.view : null));
const diffView = computed(() => (props.view.kind === "diff" ? props.view : null));
</script>

<template>
  <div class="relative min-h-0 flex-1" :aria-busy="!ready">
    <!-- The terminal section is never unmounted: xterm mis-measures from a hidden box, and
         `visible` is what tells it to hold off until the panel is really on screen. -->
    <section v-show="terminal" id="main-view-terminal" class="absolute inset-0">
      <SessionPane
        ref="sessionPane"
        class="absolute inset-0"
        :checkout="ready ? checkout : null"
        :active-session-id="activeSessionId"
        :is-opening="isOpening || !ready"
        :visible="ready && terminal"
        :shell-request="shellRequest"
        :nvim-request="nvimRequest"
        :registered-session-ids="registeredSessionIds"
        @open-folder="$emit('openFolder')"
        @workspace-updated="$emit('workspaceUpdated', $event)"
        @session-status-changed="(sessionId, status) => $emit('sessionStatusChanged', sessionId, status)"
      />
    </section>
    <section v-show="documentView" id="main-view-document" class="absolute inset-0">
      <DocumentPane
        class="absolute inset-0"
        :checkout="checkout"
        :path="documentView?.path ?? null"
        :mode="documentView?.mode ?? 'code'"
        :git-snapshot="gitSnapshot"
        :refresh-revision="refreshRevision"
        :reading-position="readingPosition"
        :zed-available="zedAvailable"
        :neovim-available="neovimAvailable"
        @update-mode="$emit('updateDocumentMode', $event)"
        @reading-position-changed="$emit('readingPositionChanged', $event)"
        @open-markdown-link="$emit('openMarkdownLink', $event)"
        @open-in-zed="$emit('openInZed')"
        @open-in-neovim="$emit('openInNeovim')"
      />
    </section>
    <section v-show="diffView" id="main-view-diff" class="absolute inset-0">
      <FileDiff
        v-if="diffView && checkout"
        class="absolute inset-0"
        :checkout="checkout"
        :git-snapshot="gitSnapshot"
        :review="review"
        :path="diffView.path"
        :active="ready"
        :scroll-top="diffScrollTop"
        :zed-available="zedAvailable"
        :neovim-available="neovimAvailable"
        @scroll-position-changed="$emit('diffPositionChanged', $event)"
        @open-in-zed="$emit('openInZed')"
        @open-in-neovim="$emit('openInNeovim')"
      />
    </section>
  </div>
</template>
