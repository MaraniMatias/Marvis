<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { Checkout } from "./domain/workspace";
import type { PaletteCommandId } from "./domain/command-palette";
import { getPaletteCommands } from "./domain/command-palette";
import { parseEditorPosition } from "./domain/editor";
import type { EditorPosition } from "./domain/editor";
import type { MainDocument, MainDocumentMode } from "./domain/main-document";
import InspectorPane from "./components/InspectorPane.vue";
import CommandPalette from "./components/CommandPalette.vue";
import DocumentPane from "./components/DocumentPane.vue";
import GitStatusBar from "./components/GitStatusBar.vue";
import SessionPane from "./components/SessionPane.vue";
import Sidebar from "./components/Sidebar.vue";
import WorktreeDialog from "./components/WorktreeDialog.vue";
import type { TerminalSessionStatus, WorkspaceState } from "./domain/workspace";
import { isIpcError } from "./domain/ipc";
import {
  closeMissingCheckout,
  getEditorAvailability,
  locateMissingCheckout,
  openInZed,
  selectCheckout as persistCheckoutSelection,
} from "./lib/ipc";
import type { EditorAvailability } from "./lib/ipc";
import { useWorkspaceState } from "./presentation/workspace";
import { useActiveGitSnapshot } from "./presentation/active-git-snapshot";
import { isMarkdownPath } from "./presentation/markdown-preview";

const {
  workspace,
  activeCheckout,
  isOpening,
  error,
  chooseFolder,
  selectCheckout,
  selectSession: selectWorkspaceSession,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const activeRepo = computed(
  () =>
    workspace.value.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === activeCheckout.value?.id)) ??
    null,
);
const gitSnapshot = useActiveGitSnapshot(activeCheckout, activeRepo, () => void promptForDefaultBranchIfNeeded(true));
const allCheckouts = computed(() => workspace.value.repos.flatMap((repo) => repo.checkouts));
const registeredSessionIds = computed(() =>
  workspace.value.repos.flatMap((repo) =>
    repo.checkouts.flatMap((checkout) => checkout.sessions.map((session) => session.id)),
  ),
);
const lifecycle = ref<{ mode: "create" | "remove"; checkoutId: string } | null>(null);
const shellRequest = ref<{ checkoutId: string; token: number } | null>(null);
const nvimRequest = ref<{
  checkoutId: string;
  filePath?: string;
  line?: number;
  column?: number;
  token: number;
} | null>(null);
const documents = ref<Record<string, MainDocument>>({});
const mainViews = ref<Record<string, "terminal" | "document">>({});
const inspectorCommand = ref<{
  action: "open-file" | "open-changes";
  token: number;
} | null>(null);
const editorAvailability = ref<EditorAvailability>({ zed: false, neovim: false });
const sessionRuntimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const recentFileWrites = ref<Record<string, boolean>>({});
const documentRefreshRevisions = ref<Record<string, number>>({});
const activityByCheckout = computed(() => {
  const activity: Record<string, string[]> = {};
  for (const checkout of allCheckouts.value) {
    const actors = checkout.sessions.flatMap((session) => {
      const status = sessionRuntimeStatuses.value[session.id];
      if (status?.state !== "running" || !status.foregroundProcess) return [];
      return [`${session.type === "nvim" ? "Neovim" : "Terminal"} · ${session.name}`];
    });
    if (recentFileWrites.value[checkout.id]) actors.push("Recent file writes");
    activity[checkout.id] = actors;
  }
  return activity;
});
let shellRequestToken = 0;
let nvimRequestToken = 0;
let inspectorCommandToken = 0;
let unlistenFileActivity: (() => void) | undefined;
let activityListenerDisposed = false;
const activityExpiryTimers = new Map<string, number>();
const lifecycleCheckout = computed<Checkout | null>(
  () =>
    workspace.value.repos
      .flatMap((repo) => repo.checkouts)
      .find((checkout) => checkout.id === lifecycle.value?.checkoutId) ?? null,
);
const lifecycleRepo = computed(
  () =>
    workspace.value.repos.find((repo) =>
      repo.checkouts.some((checkout) => checkout.id === lifecycle.value?.checkoutId),
    ) ?? null,
);
const paletteCommands = computed(() => {
  const checkout = activeCheckout.value;
  const document = checkout ? documents.value[checkout.id] : undefined;
  return getPaletteCommands({
    hasCheckout: Boolean(checkout),
    isMissing: checkout?.isMissing ?? false,
    isGit: activeRepo.value?.kind === "git",
    hasSelectedFile: Boolean(document),
    zedAvailable: editorAvailability.value.zed,
    neovimAvailable: editorAvailability.value.neovim,
  });
});
const activeDocument = computed(() => {
  const checkout = activeCheckout.value;
  const document = checkout ? documents.value[checkout.id] : undefined;
  if (!checkout || !document || document.checkoutId !== checkout.id) return null;
  return document;
});
const activeMainView = computed(() => {
  const checkoutId = activeCheckout.value?.id;
  return checkoutId && activeDocument.value && mainViews.value[checkoutId] === "document" ? "document" : "terminal";
});

function openFileDocument(selection: { checkoutId: string; path: string }) {
  documents.value = {
    ...documents.value,
    [selection.checkoutId]: {
      ...selection,
      source: "file",
      mode: isMarkdownPath(selection.path) ? "view" : "code",
    },
  };
  mainViews.value = { ...mainViews.value, [selection.checkoutId]: "document" };
}

function openChangedDocument(selection: { checkoutId: string; path: string }) {
  documents.value = { ...documents.value, [selection.checkoutId]: { ...selection, source: "change", mode: "diff" } };
  mainViews.value = { ...mainViews.value, [selection.checkoutId]: "document" };
}

function setDocumentMode(mode: MainDocumentMode) {
  const checkoutId = activeCheckout.value?.id;
  const document = activeDocument.value;
  if (!checkoutId || !document) return;
  documents.value = { ...documents.value, [checkoutId]: { ...document, mode } };
}

function closeDocument() {
  const checkoutId = activeCheckout.value?.id;
  if (!checkoutId) return;
  const next = { ...documents.value };
  delete next[checkoutId];
  documents.value = next;
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
}

function activateCheckoutTerminal(checkoutId: string) {
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
  void selectCheckout(checkoutId);
}

function activateTerminalSession(sessionId: string) {
  const checkout = allCheckouts.value.find((item) => item.sessions.some((session) => session.id === sessionId));
  if (!checkout) return;
  mainViews.value = { ...mainViews.value, [checkout.id]: "terminal" };
  void selectWorkspaceSession(sessionId);
}

onMounted(async () => {
  try {
    const dispose = await listen<string>("checkout-file-activity", (event) => {
      recentFileWrites.value = { ...recentFileWrites.value, [event.payload]: true };
      documentRefreshRevisions.value = {
        ...documentRefreshRevisions.value,
        [event.payload]: (documentRefreshRevisions.value[event.payload] ?? 0) + 1,
      };
      const previous = activityExpiryTimers.get(event.payload);
      if (previous !== undefined) window.clearTimeout(previous);
      activityExpiryTimers.set(
        event.payload,
        window.setTimeout(() => {
          const remaining = { ...recentFileWrites.value };
          delete remaining[event.payload];
          recentFileWrites.value = remaining;
          activityExpiryTimers.delete(event.payload);
        }, 5000),
      );
    });
    if (activityListenerDisposed) dispose();
    else unlistenFileActivity = dispose;
  } catch {
    // File-write activity is optional; PTY foreground-process activity remains observable.
  }
  try {
    editorAvailability.value = await getEditorAvailability();
  } catch {
    editorAvailability.value = { zed: false, neovim: false };
  }
});

onUnmounted(() => {
  activityListenerDisposed = true;
  unlistenFileActivity?.();
  for (const timer of activityExpiryTimers.values()) window.clearTimeout(timer);
  activityExpiryTimers.clear();
});

function openWorktreeDialog(mode: "create" | "remove", checkoutId: string) {
  lifecycle.value = { mode, checkoutId };
}

async function requestShell(checkoutId: string) {
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
    shellRequest.value = { checkoutId, token: ++shellRequestToken };
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}

async function requestNvim(checkoutId: string, filePath?: string, position?: EditorPosition) {
  mainViews.value = { ...mainViews.value, [checkoutId]: "terminal" };
  const request = {
    checkoutId,
    ...(filePath && position && { filePath, line: position.line, column: position.column }),
    token: ++nvimRequestToken,
  };
  nvimRequest.value = request;
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
  } catch (cause) {
    if (nvimRequest.value?.token === request.token) nvimRequest.value = null;
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}

function promptEditorPosition(): EditorPosition | null {
  const value = window.prompt("Open selected file at line[:column]:", "1");
  if (value === null) return null;
  const position = parseEditorPosition(value);
  if (!position) error.value = "Enter a positive line number with an optional column (for example, 42:7).";
  else error.value = null;
  return position;
}

function requestInspector(action: "open-file" | "open-changes") {
  inspectorCommand.value = { action, token: ++inspectorCommandToken };
}

function updateSessionStatus(sessionId: string, status: TerminalSessionStatus | null) {
  if (status) sessionRuntimeStatuses.value[sessionId] = status;
  else delete sessionRuntimeStatuses.value[sessionId];
}

async function runPaletteCommand(command: PaletteCommandId) {
  const checkout = activeCheckout.value;
  switch (command) {
    case "open-directory":
      await chooseFolder();
      break;
    case "new-worktree":
      if (activeRepo.value?.kind === "git") {
        const primary = activeRepo.value.checkouts.find((item) => item.isPrimary && !item.isMissing);
        if (primary) openWorktreeDialog("create", primary.id);
      }
      break;
    case "new-terminal":
      if (checkout && !checkout.isMissing) await requestShell(checkout.id);
      break;
    case "open-file":
      if (checkout && !checkout.isMissing) requestInspector("open-file");
      break;
    case "open-changes":
      if (checkout && activeRepo.value?.kind === "git" && !checkout.isMissing) requestInspector("open-changes");
      break;
    case "open-preview":
      if (activeDocument.value) {
        const mode = isMarkdownPath(activeDocument.value.path) ? "view" : "code";
        setDocumentMode(mode);
        mainViews.value = { ...mainViews.value, [activeDocument.value.checkoutId]: "document" };
      }
      break;
    case "open-zed":
      if (checkout && !checkout.isMissing) {
        const file = activeDocument.value?.checkoutId === checkout.id ? activeDocument.value.path : undefined;
        const position = file ? promptEditorPosition() : undefined;
        if (file && !position) break;
        try {
          await openInZed(checkout.id, file, position?.line, position?.column);
        } catch (cause) {
          error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
        }
      }
      break;
    case "open-neovim":
      if (checkout && !checkout.isMissing) {
        const file = activeDocument.value?.checkoutId === checkout.id ? activeDocument.value.path : undefined;
        const position = file ? promptEditorPosition() : undefined;
        if (file && !position) break;
        await requestNvim(checkout.id, file, position ?? undefined);
      }
      break;
  }
}

function applyWorkspace(next: WorkspaceState) {
  updateWorkspace(next);
}

function reportWarning(message: string) {
  error.value = message;
}

async function locateCheckout(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  if (!checkout) return;
  error.value = null;
  isOpening.value = true;
  try {
    const parent = checkout.path.replace(/[\\/][^\\/]*$/, "") || "/";
    const path = await open({
      directory: true,
      multiple: false,
      title: "Locate missing checkout",
      defaultPath: parent,
    });
    if (typeof path === "string") workspace.value = await locateMissingCheckout(checkoutId, path);
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  } finally {
    isOpening.value = false;
  }
}

async function closeCheckout(checkoutId: string) {
  const checkout = allCheckouts.value.find((item) => item.id === checkoutId);
  const repo = workspace.value.repos.find((item) => item.checkouts.some((entry) => entry.id === checkoutId));
  if (!checkout || !repo) return;
  const closesRepo = checkout.isPrimary || repo.kind === "plain";
  const scope = closesRepo ? `“${repo.name}” and its checkout list` : `“${checkout.path}”`;
  if (!window.confirm(`Close ${scope} in Marvis? No files will be deleted.`)) return;
  try {
    workspace.value = await closeMissingCheckout(checkoutId);
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}
</script>

<template>
  <div class="relative flex h-full min-w-[900px] flex-col bg-[#111318] text-zinc-100">
    <div class="flex min-h-0 flex-1">
      <Sidebar
        :repos="workspace.repos"
        :active-checkout-id="workspace.activeCheckoutId"
        :active-session-id="workspace.activeSessionId"
        :activity-by-checkout="activityByCheckout"
        :session-runtime-statuses="sessionRuntimeStatuses"
        :is-opening="isOpening"
        @open-folder="chooseFolder"
        @select-checkout="activateCheckoutTerminal"
        @select-session="activateTerminalSession"
        @locate-missing="locateCheckout"
        @close-missing="closeCheckout"
        @create-worktree="openWorktreeDialog('create', $event)"
        @remove-worktree="openWorktreeDialog('remove', $event)"
      />
      <div class="flex min-w-0 flex-1 flex-col">
        <nav
          role="tablist"
          aria-label="Main view"
          class="flex h-11 shrink-0 items-center gap-1 border-b border-white/8 px-3"
        >
          <button
            role="tab"
            type="button"
            :aria-selected="activeMainView === 'terminal'"
            class="rounded px-3 py-1.5 text-xs"
            :class="activeMainView === 'terminal' ? 'bg-white/8 text-zinc-100' : 'text-zinc-500 hover:text-zinc-300'"
            @click="activeCheckout && (mainViews = { ...mainViews, [activeCheckout.id]: 'terminal' })"
          >
            Terminal
          </button>
          <div
            v-if="activeDocument"
            class="flex h-full items-center gap-1 border-b px-2"
            :class="activeMainView === 'document' ? 'border-sky-400/60' : 'border-transparent'"
          >
            <button
              role="tab"
              type="button"
              :aria-selected="activeMainView === 'document'"
              class="max-w-64 truncate px-1 py-1.5 text-xs"
              :class="activeMainView === 'document' ? 'text-zinc-100' : 'text-zinc-500 hover:text-zinc-300'"
              @click="mainViews = { ...mainViews, [activeDocument.checkoutId]: 'document' }"
            >
              {{ activeDocument.path.split(/[\\/]/).at(-1) }}
            </button>
            <button
              type="button"
              aria-label="Close document"
              class="rounded px-1 text-zinc-500 hover:bg-white/8 hover:text-zinc-200"
              @click="closeDocument"
            >
              ×
            </button>
          </div>
        </nav>
        <div class="relative min-h-0 flex-1">
          <SessionPane
            v-show="activeMainView === 'terminal'"
            class="absolute inset-0"
            :checkout="activeCheckout"
            :active-session-id="workspace.activeSessionId"
            :is-opening="isOpening"
            :visible="activeMainView === 'terminal'"
            :shell-request="shellRequest"
            :nvim-request="nvimRequest"
            :registered-session-ids="registeredSessionIds"
            @open-folder="chooseFolder"
            @workspace-updated="updateWorkspace"
            @session-status-changed="updateSessionStatus"
          />
          <DocumentPane
            v-if="activeDocument"
            v-show="activeMainView === 'document'"
            class="absolute inset-0"
            :checkout="activeCheckout"
            :document="activeDocument"
            :git-snapshot="gitSnapshot"
            :active="activeMainView === 'document'"
            :refresh-revision="documentRefreshRevisions[activeDocument.checkoutId] ?? 0"
            @update-mode="setDocumentMode"
          />
        </div>
      </div>
      <InspectorPane
        :checkout="activeCheckout"
        :repo="activeRepo"
        :git-snapshot="gitSnapshot"
        :command-request="inspectorCommand"
        @open-file="openFileDocument"
        @open-change="openChangedDocument"
      />
    </div>
    <WorktreeDialog
      :open="!!lifecycle"
      :mode="lifecycle?.mode ?? 'create'"
      :repo="lifecycleRepo"
      :checkout="lifecycleCheckout"
      @close="lifecycle = null"
      @workspace-updated="applyWorkspace"
      @request-shell="requestShell"
      @warning="reportWarning"
    />
    <GitStatusBar
      :checkout="activeCheckout"
      :repo="activeRepo"
      :git-snapshot="gitSnapshot"
      :concurrent-actors="activeCheckout ? activityByCheckout[activeCheckout.id] : []"
    />
    <CommandPalette :commands="paletteCommands" @select="runPaletteCommand" />
    <div
      v-if="error"
      role="alert"
      class="absolute bottom-12 left-1/2 max-w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 rounded-lg border border-red-400/20 bg-[#242126] px-4 py-3 text-sm text-red-200 shadow-xl"
    >
      {{ error }}
    </div>
  </div>
</template>
