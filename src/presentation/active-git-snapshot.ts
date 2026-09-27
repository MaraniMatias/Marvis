import { listen } from "@tauri-apps/api/event";
import { reactive, watch } from "vue";
import type { ComputedRef } from "vue";
import type { GitStatus } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import type { Checkout, Repo } from "../domain/workspace";
import { getGitStatus, markGitFileViewed, unwatchGitCheckout, watchGitCheckout } from "../lib/ipc";

export interface ActiveGitSnapshot {
  checkoutId: string | null;
  status: GitStatus | null;
  loading: boolean;
  statusState: "loading" | "ready" | "error";
  statusError: string;
  changesStatusError: string;
  viewedError: string;
  changesWatchError: string;
  statusRevision: number;
  statusEventRevision: number;
  statusEventCheckoutId: string | null;
  markViewed(checkoutId: string, path: string): Promise<void>;
}

function errorText(cause: unknown): string {
  return isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}

export function useActiveGitSnapshot(
  checkout: ComputedRef<Checkout | null>,
  repo: ComputedRef<Repo | null>,
  onDefaultBranchUnknown: () => void,
): ActiveGitSnapshot {
  let generation = 0;
  let requestedDefaultBranch = false;
  let refreshCurrentStatus: (() => Promise<void>) | undefined;
  let refreshCurrentStatusGeneration = 0;
  const watcherOperations = new Map<string, Promise<void>>();
  const state = reactive<Omit<ActiveGitSnapshot, "markViewed">>({
    checkoutId: null,
    status: null,
    loading: false,
    statusState: "ready",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
  });

  function queueWatcherOperation(checkoutId: string, operation: () => Promise<void>): Promise<void> {
    const previous = watcherOperations.get(checkoutId) ?? Promise.resolve();
    const queued = previous.catch(() => undefined).then(operation);
    watcherOperations.set(checkoutId, queued);
    queued.then(
      () => {
        if (watcherOperations.get(checkoutId) === queued) watcherOperations.delete(checkoutId);
      },
      () => {
        if (watcherOperations.get(checkoutId) === queued) watcherOperations.delete(checkoutId);
      },
    );
    return queued;
  }

  /** E.7: nothing in the UI tracks "viewed" anymore. `FileDiff` still reports it to the
   *  backend, so the call and its error stay until the diff view owns them (phase 6). */
  async function markViewed(checkoutId: string, path: string) {
    const requestGeneration = generation;
    try {
      await markGitFileViewed(checkoutId, path);
      if (requestGeneration === generation && state.checkoutId === checkoutId) state.viewedError = "";
    } catch (cause) {
      if (requestGeneration === generation && state.checkoutId === checkoutId) state.viewedError = errorText(cause);
    }
  }

  watch(
    [() => checkout.value?.id, () => checkout.value?.isMissing, () => repo.value?.kind],
    async ([checkoutId, isMissing, repoKind], _previous, onCleanup) => {
      const requestGeneration = ++generation;
      let current = true;
      let watchQueued = false;
      let unlistenStatus: (() => void) | undefined;
      let statusRequest = 0;
      const isCurrent = () => current && requestGeneration === generation;

      onCleanup(() => {
        current = false;
        generation += 1;
        if (refreshCurrentStatusGeneration === requestGeneration) refreshCurrentStatus = undefined;
        unlistenStatus?.();
        if (watchQueued && checkoutId) {
          void queueWatcherOperation(checkoutId, () => unwatchGitCheckout(checkoutId)).catch(() => undefined);
        }
      });

      state.checkoutId = checkoutId ?? null;
      state.status = null;
      state.loading = false;
      state.statusState = "ready";
      state.statusError = "";
      state.changesStatusError = "";
      state.viewedError = "";
      state.changesWatchError = "";
      state.statusEventCheckoutId = null;
      requestedDefaultBranch = false;
      if (!checkoutId) return;
      if (isMissing) {
        state.statusError = "Directory missing";
        state.changesStatusError = "Directory missing";
        state.statusState = "error";
        return;
      }
      if (repoKind !== "git") return;

      state.loading = true;
      state.statusState = "loading";

      const refreshStatus = async () => {
        const refreshRequest = ++statusRequest;
        state.changesStatusError = "";
        if (!state.status) state.statusState = "loading";
        try {
          const result = await getGitStatus(checkoutId);
          if (!isCurrent() || refreshRequest !== statusRequest) return;
          state.status = result;
          state.statusError = "";
          state.changesStatusError = "";
          state.statusState = "ready";
          state.statusRevision += 1;
        } catch (cause) {
          if (!isCurrent() || refreshRequest !== statusRequest) return;
          state.status = null;
          state.statusError = errorText(cause);
          state.changesStatusError = errorText(cause);
          state.statusState = "error";
          if (isIpcError(cause) && cause.code === "default_branch_unknown" && !requestedDefaultBranch) {
            requestedDefaultBranch = true;
            onDefaultBranchUnknown();
          }
        } finally {
          if (isCurrent() && refreshRequest === statusRequest) state.loading = false;
        }
      };
      refreshCurrentStatus = refreshStatus;
      refreshCurrentStatusGeneration = requestGeneration;

      try {
        const dispose = await listen<string>("git-status-changed", (event) => {
          if (event.payload === checkoutId && isCurrent()) {
            state.statusEventCheckoutId = checkoutId;
            state.statusEventRevision += 1;
            void refreshStatus();
          }
        });
        if (!isCurrent()) {
          dispose();
          return;
        }
        unlistenStatus = dispose;
      } catch (cause) {
        if (isCurrent()) state.changesWatchError = errorText(cause);
      }
      if (!isCurrent()) return;

      watchQueued = true;
      try {
        await queueWatcherOperation(checkoutId, async () => {
          if (isCurrent()) await watchGitCheckout(checkoutId);
        });
      } catch (cause) {
        if (isCurrent()) state.changesWatchError = errorText(cause);
      }
      if (isCurrent()) await refreshStatus();
    },
    { immediate: true },
  );

  watch(
    [() => repo.value?.defaultBranch, () => checkout.value?.id],
    ([branch, checkoutId], [previousBranch, previousCheckoutId]) => {
      if (
        checkoutId &&
        checkoutId === previousCheckoutId &&
        branch !== previousBranch &&
        repo.value?.kind === "git" &&
        !checkout.value?.isMissing
      ) {
        requestedDefaultBranch = false;
        void refreshCurrentStatus?.();
      }
    },
  );

  return Object.assign(state, { markViewed });
}
