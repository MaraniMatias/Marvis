<script setup lang="ts">
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { Checkout } from "./domain/workspace";
import type { PaletteCommandId } from "./domain/command-palette";
import { getPaletteCommands } from "./domain/command-palette";
import { parseEditorPosition } from "./domain/editor";
import type { EditorPosition } from "./domain/editor";
import InspectorPane from "./components/InspectorPane.vue";
import CommandPalette from "./components/CommandPalette.vue";
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

const {
  workspace,
  activeCheckout,
  isOpening,
  error,
  chooseFolder,
  selectCheckout,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const activeRepo = computed(
  () =>
    workspace.value.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === activeCheckout.value?.id)) ??
    null,
);
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
const selectedFile = ref<{ checkoutId: string; path: string } | null>(null);
const inspectorCommand = ref<{
  action: "open-file" | "open-changes" | "open-preview";
  token: number;
} | null>(null);
const editorAvailability = ref<EditorAvailability>({ zed: false, neovim: false });
const sessionRuntimeStatuses = ref<Record<string, TerminalSessionStatus>>({});
const recentFileWrites = ref<Record<string, boolean>>({});
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
  return getPaletteCommands({
    hasCheckout: Boolean(checkout),
    isMissing: checkout?.isMissing ?? false,
    isGit: activeRepo.value?.kind === "git",
    hasSelectedFile: selectedFile.value?.checkoutId === checkout?.id,
    zedAvailable: editorAvailability.value.zed,
    neovimAvailable: editorAvailability.value.neovim,
  });
});

onMounted(async () => {
  try {
    const dispose = await listen<string>("checkout-file-activity", (event) => {
      recentFileWrites.value = { ...recentFileWrites.value, [event.payload]: true };
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
  try {
    workspace.value = await persistCheckoutSelection(checkoutId);
    shellRequest.value = { checkoutId, token: ++shellRequestToken };
  } catch (cause) {
    error.value = isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
  }
}

async function requestNvim(checkoutId: string, filePath?: string, position?: EditorPosition) {
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

function requestInspector(action: "open-file" | "open-changes" | "open-preview") {
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
      if (selectedFile.value?.checkoutId === checkout?.id) requestInspector("open-preview");
      break;
    case "open-zed":
      if (checkout && !checkout.isMissing) {
        const file = selectedFile.value?.checkoutId === checkout.id ? selectedFile.value.path : undefined;
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
        const file = selectedFile.value?.checkoutId === checkout.id ? selectedFile.value.path : undefined;
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

function handleDefaultBranchUnknown() {
  void promptForDefaultBranchIfNeeded(true);
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
        @select-checkout="selectCheckout"
        @locate-missing="locateCheckout"
        @close-missing="closeCheckout"
        @create-worktree="openWorktreeDialog('create', $event)"
        @remove-worktree="openWorktreeDialog('remove', $event)"
      />
      <SessionPane
        :checkout="activeCheckout"
        :active-session-id="workspace.activeSessionId"
        :is-opening="isOpening"
        :shell-request="shellRequest"
        :nvim-request="nvimRequest"
        :registered-session-ids="registeredSessionIds"
        @open-folder="chooseFolder"
        @workspace-updated="updateWorkspace"
        @session-status-changed="updateSessionStatus"
      />
      <InspectorPane
        :checkout="activeCheckout"
        :repo="activeRepo"
        :command-request="inspectorCommand"
        @default-branch-unknown="handleDefaultBranchUnknown"
        @selected-file="selectedFile = $event"
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
      :concurrent-actors="activeCheckout ? activityByCheckout[activeCheckout.id] : []"
      @default-branch-unknown="handleDefaultBranchUnknown"
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
