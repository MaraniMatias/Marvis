<script setup lang="ts">
/* eslint-disable vue/html-self-closing, vue/html-closing-bracket-newline, vue/html-indent */
import { computed, ref, watch } from "vue";
import type { Checkout, Repo, WorkspaceState } from "../domain/workspace";
import type { WorktreeRemovalInfo } from "../domain/worktree";
import { isIpcError } from "../domain/ipc";
import { createWorktree, getWorktreeDefaults, getWorktreeRemovalInfo, removeWorktree } from "../lib/ipc";

const props = defineProps<{
  open: boolean;
  mode: "create" | "remove";
  repo: Repo | null;
  checkout: Checkout | null;
}>();

const emit = defineEmits<{
  close: [];
  workspaceUpdated: [workspace: WorkspaceState];
  requestShell: [checkoutId: string];
  warning: [message: string];
}>();

const loading = ref(false);
const error = ref<string | null>(null);
const location = ref("");
const taskName = ref("new-task");
const branch = ref("feature/new-task");
const defaultBranch = ref("");
const branchEdited = ref(false);
const removal = ref<WorktreeRemovalInfo | null>(null);
const dirtyConfirmed = ref(false);
const sessionsConfirmed = ref(false);
const branchAction = ref<"keep" | "delete">("keep");
const isBusy = computed(() => loading.value);
const canRemove = computed(
  () =>
    !!removal.value &&
    removal.value.activeAgentSessions.length === 0 &&
    (removal.value.dirtyFiles.length === 0 || dirtyConfirmed.value) &&
    (removal.value.activeSessions.length === 0 || sessionsConfirmed.value),
);

watch(
  () => [props.open, props.mode, props.checkout?.id] as const,
  async ([open]) => {
    if (!open || !props.checkout) return;
    error.value = null;
    loading.value = true;
    removal.value = null;
    dirtyConfirmed.value = false;
    sessionsConfirmed.value = false;
    branchAction.value = "keep";
    try {
      if (props.mode === "create") {
        taskName.value = "new-task";
        branch.value = "feature/new-task";
        branchEdited.value = false;
        await loadDefaults();
      } else {
        removal.value = await getWorktreeRemovalInfo(props.checkout.id);
      }
    } catch (cause) {
      error.value = messageOf(cause);
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

async function loadDefaults() {
  if (!props.checkout || !props.repo) return;
  const defaults = await getWorktreeDefaults(props.checkout.id);
  location.value = defaults.location;
  defaultBranch.value = defaults.defaultBranch;
}

function updateTaskName(value: string) {
  taskName.value = value;
  if (branchEdited.value) return;
  const slug =
    value
      .trim()
      .replace(/[^\p{L}\p{N}_.-]+/gu, "-")
      .replace(/^-+|-+$/g, "")
      .replace(/^\.+/, "") || "new-task";
  branch.value = `feature/${slug}`;
}

async function submitCreate() {
  if (!props.checkout || !taskName.value.trim() || !branch.value.trim() || !location.value.trim()) return;
  loading.value = true;
  error.value = null;
  try {
    const result = await createWorktree(props.checkout.id, taskName.value, branch.value, location.value);
    emit("workspaceUpdated", result.workspace);
    emit("close");
  } catch (cause) {
    error.value = messageOf(cause);
  } finally {
    loading.value = false;
  }
}

async function submitRemove() {
  if (!props.checkout || !removal.value || !canRemove.value) return;
  loading.value = true;
  error.value = null;
  try {
    const result = await removeWorktree(
      props.checkout.id,
      removal.value.dirtyFiles.length > 0 && dirtyConfirmed.value,
      removal.value.dirtyFiles,
      removal.value.activeSessions.map((session) => session.id),
      removal.value.branch,
      removal.value.unmergedCommits,
      branchAction.value === "delete",
    );
    emit("workspaceUpdated", result.workspace);
    if (result.warning) emit("warning", result.warning);
    emit("close");
  } catch (cause) {
    error.value = messageOf(cause);
    if (props.checkout && isIpcError(cause) && cause.code === "invalid_checkout") {
      try {
        removal.value = await getWorktreeRemovalInfo(props.checkout.id);
        dirtyConfirmed.value = false;
        sessionsConfirmed.value = false;
      } catch {
        // Preserve the removal error; reopening the dialog can refresh a stale checkout.
      }
    }
  } finally {
    loading.value = false;
  }
}

function openShell() {
  if (!props.checkout) return;
  emit("close");
  emit("requestShell", props.checkout.id);
}

function messageOf(cause: unknown): string {
  return isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}
</script>

<template>
  <div v-if="open" class="fixed inset-0 z-50 grid place-items-center bg-black/65 p-4" @click.self="$emit('close')">
    <section
      role="dialog"
      aria-modal="true"
      :aria-labelledby="mode === 'create' ? 'worktree-create-title' : 'worktree-remove-title'"
      class="w-full max-w-xl rounded-xl border border-white/10 bg-[#191b21] p-5 shadow-2xl"
    >
      <header class="mb-4 flex items-start justify-between gap-4">
        <div>
          <h2
            :id="mode === 'create' ? 'worktree-create-title' : 'worktree-remove-title'"
            class="text-base font-semibold text-zinc-100"
          >
            {{ mode === "create" ? "Create worktree from main" : "Remove worktree" }}
          </h2>
          <p class="mt-1 truncate text-xs text-zinc-500">{{ checkout?.branch || checkout?.path }} · {{ repo?.name }}</p>
        </div>
        <button aria-label="Close" class="rounded px-2 py-1 text-zinc-500 hover:bg-white/8" @click="$emit('close')">
          ×
        </button>
      </header>

      <div v-if="loading && !removal" class="py-6 text-sm text-zinc-500" role="status">Checking Git worktree…</div>

      <form v-else-if="mode === 'create'" class="space-y-4" @submit.prevent="submitCreate">
        <label class="block text-xs text-zinc-400">
          Task name
          <input
            :value="taskName"
            autofocus
            required
            maxlength="120"
            class="mt-1.5 w-full rounded-md border border-white/10 bg-[#101217] px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-300/50"
            @input="updateTaskName(($event.target as HTMLInputElement).value)"
          />
          <span class="mt-1 block text-[11px] text-zinc-600"
            >Used as a directory name; unsafe characters are cleaned by Git service validation.</span
          >
        </label>
        <label class="block text-xs text-zinc-400">
          New branch
          <input
            v-model="branch"
            required
            class="mt-1.5 w-full rounded-md border border-white/10 bg-[#101217] px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-300/50"
            @input="branchEdited = true"
          />
        </label>
        <p v-if="defaultBranch" class="text-xs text-zinc-500">
          Starting point: <code>{{ defaultBranch }}</code> · Worktrees are created under <code>{{ location }}</code
          >.
        </p>
        <p v-if="error" role="alert" class="text-sm text-red-200">{{ error }}</p>
        <footer class="flex justify-end gap-2 pt-1">
          <button
            type="button"
            class="rounded-md px-3 py-2 text-xs text-zinc-400 hover:bg-white/6"
            @click="$emit('close')"
          >
            Cancel
          </button>
          <button
            type="submit"
            :disabled="isBusy || !defaultBranch"
            class="rounded-md bg-indigo-300 px-3 py-2 text-xs font-semibold text-[#111318] disabled:opacity-50"
          >
            {{ isBusy ? "Creating…" : "Create and open shell" }}
          </button>
        </footer>
      </form>

      <div v-else-if="mode === 'remove' && error && !removal" class="space-y-4">
        <p role="alert" class="text-sm text-red-200">{{ error }}</p>
        <button
          type="button"
          class="rounded-md px-3 py-2 text-xs text-zinc-400 hover:bg-white/6"
          @click="$emit('close')"
        >
          Close
        </button>
      </div>

      <div v-else-if="removal" class="space-y-4">
        <p
          v-if="removal.isMissing"
          class="rounded-md border border-amber-300/20 bg-amber-300/5 p-3 text-sm text-amber-100"
        >
          This checkout is missing. Git will prune its stale worktree metadata and remove it from Marvis.
        </p>
        <div v-if="removal.dirtyFiles.length" class="rounded-md border border-amber-300/20 bg-amber-300/5 p-3">
          <p class="text-sm text-amber-100">
            {{ removal.dirtyFiles.length }} uncommitted change{{ removal.dirtyFiles.length === 1 ? "" : "s" }} will be
            discarded if you continue.
          </p>
          <ul class="mt-2 max-h-24 overflow-y-auto pl-4 text-xs text-amber-100/70">
            <li v-for="file in removal.dirtyFiles.slice(0, 8)" :key="file" class="truncate">{{ file }}</li>
            <li v-if="removal.dirtyFiles.length > 8">and {{ removal.dirtyFiles.length - 8 }} more…</li>
          </ul>
          <button type="button" class="mt-2 text-xs text-indigo-200 underline" @click="openShell">
            Open a shell to commit or stash first
          </button>
          <label class="mt-3 flex items-start gap-2 text-xs text-amber-100/80">
            <input v-model="dirtyConfirmed" type="checkbox" class="mt-0.5 accent-amber-300" />
            I understand uncommitted files may be lost; remove anyway.
          </label>
        </div>
        <p
          v-if="removal.unmergedCommits"
          class="rounded-md border border-amber-300/20 bg-amber-300/5 p-3 text-sm text-amber-100"
        >
          This branch has {{ removal.unmergedCommits }} commit{{ removal.unmergedCommits === 1 ? "" : "s" }} not merged
          into the default branch. Keeping the branch preserves them.
        </p>
        <div v-if="removal.activeSessions.length" class="rounded-md border border-white/10 bg-black/10 p-3">
          <p class="text-sm text-zinc-200">These active sessions will be stopped:</p>
          <ul class="mt-2 space-y-1 text-xs text-zinc-400">
            <li v-for="session in removal.activeSessions" :key="session.id">{{ session.name }} · {{ session.type }}</li>
          </ul>
          <label class="mt-3 flex items-start gap-2 text-xs text-zinc-400">
            <input v-model="sessionsConfirmed" type="checkbox" class="mt-0.5 accent-indigo-300" />
            Stop the listed sessions and remove this checkout.
          </label>
        </div>
        <p
          v-if="removal.activeAgentSessions.length"
          role="alert"
          class="rounded-md bg-red-400/10 p-3 text-sm text-red-200"
        >
          An active agent is using this checkout. Stop the agent before removing it:
          {{ removal.activeAgentSessions.map((session) => session.name).join(", ") }}.
        </p>
        <label v-if="removal.branch" class="block text-xs text-zinc-400">
          Local branch
          <select
            v-model="branchAction"
            class="mt-1.5 w-full rounded-md border border-white/10 bg-[#101217] px-3 py-2 text-sm text-zinc-100"
          >
            <option value="keep">Keep branch (recommended)</option>
            <option value="delete">
              Delete branch{{ removal.unmergedCommits ? " and its unmerged commits" : "" }}
            </option>
          </select>
        </label>
        <p v-if="error" role="alert" class="text-sm text-red-200">{{ error }}</p>
        <footer class="flex justify-end gap-2 pt-1">
          <button
            type="button"
            class="rounded-md px-3 py-2 text-xs text-zinc-400 hover:bg-white/6"
            @click="$emit('close')"
          >
            Cancel
          </button>
          <button
            type="button"
            :disabled="isBusy || !canRemove"
            class="rounded-md bg-red-400/90 px-3 py-2 text-xs font-semibold text-[#1a1010] disabled:opacity-50"
            @click="submitRemove"
          >
            {{ isBusy ? "Removing…" : "Remove worktree" }}
          </button>
        </footer>
      </div>
    </section>
  </div>
</template>
