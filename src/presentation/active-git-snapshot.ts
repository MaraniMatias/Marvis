import { listen } from "@tauri-apps/api/event";
import { reactive, watch } from "vue";
import type { ComputedRef } from "vue";
import type { GitStatus } from "../domain/git";
import { isIpcError } from "../domain/ipc";
import type { Checkout, Repo } from "../domain/workspace";
import { getGitStatus, getGitViewedFiles, markGitFileViewed, unwatchGitCheckout, watchGitCheckout } from "../lib/ipc";

export interface ActiveGitSnapshot {
  checkoutId: string | null;
  status: GitStatus | null;
  viewedPaths: string[];
  loading: boolean;
  statusState: "loading" | "ready" | "error";
  statusError: string;
  changesStatusError: string;
  viewedError: string;
  watchError: string;
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
    viewedPaths: [],
    loading: false,
    statusState: "ready",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
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

  async function markViewed(checkoutId: string, path: string) {
    const requestGeneration = generation;
    if (state.checkoutId !== checkoutId || state.viewedPaths.includes(path)) return;
    try {
      await markGitFileViewed(checkoutId, path);
      if (requestGeneration === generation && state.checkoutId === checkoutId && !state.viewedPaths.includes(path)) {
        state.viewedPaths = [...state.viewedPaths, path];
        state.viewedError = "";
      }
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
      let unlistenViewed: (() => void) | undefined;
      let statusRequest = 0;
      let viewedRequest = 0;
      const isCurrent = () => current && requestGeneration === generation;

      onCleanup(() => {
        current = false;
        generation += 1;
        if (refreshCurrentStatusGeneration === requestGeneration) refreshCurrentStatus = undefined;
        unlistenStatus?.();
        unlistenViewed?.();
        if (watchQueued && checkoutId) {
          void queueWatcherOperation(checkoutId, () => unwatchGitCheckout(checkoutId)).catch(() => undefined);
        }
      });

      state.checkoutId = checkoutId ?? null;
      state.status = null;
      state.viewedPaths = [];
      state.loading = false;
      state.statusState = "ready";
      state.statusError = "";
      state.changesStatusError = "";
      state.viewedError = "";
      state.watchError = "";
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

      const refreshViewed = async (expectedStatusRequest?: number) => {
        const viewedGeneration = ++viewedRequest;
        state.viewedError = "";
        try {
          const paths = await getGitViewedFiles(checkoutId);
          if (
            isCurrent() &&
            viewedGeneration === viewedRequest &&
            (expectedStatusRequest === undefined || expectedStatusRequest === statusRequest)
          ) {
            state.viewedPaths = paths;
          }
        } catch (cause) {
          if (
            isCurrent() &&
            viewedGeneration === viewedRequest &&
            (expectedStatusRequest === undefined || expectedStatusRequest === statusRequest)
          ) {
            state.viewedPaths = [];
            state.viewedError = errorText(cause);
          }
        }
      };

      const refreshStatus = async () => {
        const refreshRequest = ++statusRequest;
        viewedRequest += 1;
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
          await refreshViewed(refreshRequest);
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
        if (isCurrent()) {
          state.watchError = errorText(cause);
          state.changesWatchError = errorText(cause);
        }
      }
      if (!isCurrent()) return;

      try {
        const dispose = await listen<string>("git-viewed-changed", (event) => {
          if (event.payload === checkoutId && isCurrent()) void refreshViewed();
        });
        if (!isCurrent()) {
          dispose();
          return;
        }
        unlistenViewed = dispose;
      } catch (cause) {
        if (isCurrent()) {
          state.watchError = errorText(cause);
          state.viewedError = errorText(cause);
        }
      }
      if (!isCurrent()) return;

      watchQueued = true;
      try {
        await queueWatcherOperation(checkoutId, async () => {
          if (isCurrent()) await watchGitCheckout(checkoutId);
        });
      } catch (cause) {
        if (isCurrent()) state.watchError = errorText(cause);
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
