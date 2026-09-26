// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { computed, defineComponent, h, ref } from "vue";
import type { Ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitStatus: vi.fn(),
  markGitFileViewed: vi.fn(),
  watchGitCheckout: vi.fn(),
  unwatchGitCheckout: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
  handlers: new Map<string, (event: { payload: string }) => void>(),
}));

vi.mock("../lib/ipc", () => ({
  getGitStatus: mocks.getGitStatus,
  markGitFileViewed: mocks.markGitFileViewed,
  watchGitCheckout: mocks.watchGitCheckout,
  unwatchGitCheckout: mocks.unwatchGitCheckout,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

import { useActiveGitSnapshot } from "./active-git-snapshot";

function checkout(id: string): Checkout {
  return {
    id,
    repoId: "repo:test",
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing: false,
    sessions: [],
  };
}

function repo(defaultBranch = "trunk"): Repo {
  return {
    id: "repo:test",
    kind: "git",
    name: "test",
    root: "/test",
    defaultBranch,
    checkouts: [],
    createdAt: "now",
    lastOpenedAt: "now",
  };
}

function status(branch: string): GitStatus {
  return { branch, defaultBranch: "trunk", aheadCount: 0, files: [] };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}

function host(
  checkoutRef: Ref<Checkout | null>,
  repoRef: Ref<Repo | null>,
  onDefaultBranchUnknown: () => void = () => undefined,
) {
  return defineComponent({
    setup() {
      const snapshot = useActiveGitSnapshot(
        computed(() => checkoutRef.value),
        computed(() => repoRef.value),
        onDefaultBranchUnknown,
      );
      return () => h("div", snapshot.status?.branch ?? "no status");
    },
  });
}

describe("useActiveGitSnapshot", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.handlers.clear();
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: string }) => void) => {
      mocks.handlers.set(name, handler);
      return mocks.unlisten;
    });
    mocks.getGitStatus.mockImplementation(async (id: string) => status(id));
    mocks.watchGitCheckout.mockResolvedValue(undefined);
    mocks.unwatchGitCheckout.mockResolvedValue(undefined);
  });

  it("owns one listener and watcher pair for the selected checkout and releases them on selection change/unmount", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("first"));
    const activeRepo = ref<Repo | null>(repo());
    const wrapper = mount(host(activeCheckout, activeRepo));
    await flushPromises();

    expect(mocks.listen).toHaveBeenCalledTimes(1);
    expect(mocks.watchGitCheckout).toHaveBeenCalledTimes(1);
    expect(mocks.watchGitCheckout).toHaveBeenCalledWith("first");
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("first");

    mocks.handlers.get("git-status-changed")?.({ payload: "not-active" });
    await flushPromises();
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(1);

    mocks.handlers.get("git-status-changed")?.({ payload: "first" });
    await flushPromises();
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(2);

    activeCheckout.value = checkout("second");
    await flushPromises();
    expect(mocks.unwatchGitCheckout).toHaveBeenCalledWith("first");
    expect(mocks.watchGitCheckout).toHaveBeenLastCalledWith("second");
    expect(mocks.listen).toHaveBeenCalledTimes(2);
    expect(mocks.unlisten).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("second");

    wrapper.unmount();
    await flushPromises();
    expect(mocks.unwatchGitCheckout).toHaveBeenLastCalledWith("second");
    expect(mocks.unlisten).toHaveBeenCalledTimes(2);
  });

  it("ignores a status response that finishes after switching checkout", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("first"));
    const activeRepo = ref<Repo | null>(repo());
    let resolveFirst!: (value: GitStatus) => void;
    mocks.getGitStatus.mockImplementation((id: string) =>
      id === "first" ? new Promise<GitStatus>((resolve) => (resolveFirst = resolve)) : Promise.resolve(status(id)),
    );
    const wrapper = mount(host(activeCheckout, activeRepo));
    await flushPromises();

    activeCheckout.value = checkout("second");
    await flushPromises();
    expect(wrapper.text()).toContain("second");

    resolveFirst(status("stale-first"));
    await flushPromises();
    expect(wrapper.text()).toContain("second");
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(2);
    expect(mocks.getGitStatus).toHaveBeenLastCalledWith("second");
    wrapper.unmount();
  });

  it("does not advance to later registration phases when git-status-changed rejects after checkout change", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("first"));
    const activeRepo = ref<Repo | null>(repo());
    let rejectDelayed!: (cause: unknown) => void;
    let delayed = false;
    mocks.listen.mockImplementation(async (_name: string, handler: (event: { payload: string }) => void) => {
      if (!delayed) {
        delayed = true;
        return await new Promise<() => void>((_resolve, reject) => (rejectDelayed = reject));
      }
      mocks.handlers.set("git-status-changed", handler);
      return mocks.unlisten;
    });

    const wrapper = mount(host(activeCheckout, activeRepo));
    await flushPromises();
    activeCheckout.value = checkout("second");
    await flushPromises();
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["second"]);
    expect(mocks.getGitStatus.mock.calls.map(([id]) => id)).toEqual(["second"]);

    rejectDelayed(new Error("late listener registration failure"));
    await flushPromises();
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["second"]);
    expect(mocks.getGitStatus.mock.calls.map(([id]) => id)).toEqual(["second"]);
    expect(mocks.listen).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("serializes same-checkout teardown/start across A→B→A transitions", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("A"));
    const activeRepo = ref<Repo | null>(repo());
    const firstWatchA = deferred<void>();
    const firstUnwatchA = deferred<void>();
    const backendWatchers = new Set<string>();
    let watchACount = 0;
    let unwatchACount = 0;
    mocks.watchGitCheckout.mockImplementation((id: string) => {
      if (id === "A" && ++watchACount === 1) {
        return firstWatchA.promise.then(() => {
          backendWatchers.add(id);
        });
      }
      backendWatchers.add(id);
      return Promise.resolve();
    });
    mocks.unwatchGitCheckout.mockImplementation((id: string) => {
      if (id === "A" && ++unwatchACount === 1) {
        return firstUnwatchA.promise.then(() => {
          backendWatchers.delete(id);
        });
      }
      backendWatchers.delete(id);
      return Promise.resolve();
    });

    const wrapper = mount(host(activeCheckout, activeRepo));
    await flushPromises();
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["A"]);

    activeCheckout.value = checkout("B");
    await flushPromises();
    activeCheckout.value = checkout("A");
    await flushPromises();
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["A", "B"]);

    firstWatchA.resolve(undefined);
    await flushPromises();
    expect(mocks.unwatchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["B", "A"]);
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["A", "B"]);

    firstUnwatchA.resolve(undefined);
    await flushPromises();
    expect(mocks.watchGitCheckout.mock.calls.map(([id]) => id)).toEqual(["A", "B", "A"]);
    expect([...backendWatchers]).toEqual(["A"]);

    wrapper.unmount();
    await flushPromises();
    expect([...backendWatchers]).toEqual([]);
  });

  it("unwatches after a pending start resolves when the owner unmounts", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("A"));
    const activeRepo = ref<Repo | null>(repo());
    const pendingWatch = deferred<void>();
    const backendWatchers = new Set<string>();
    mocks.watchGitCheckout.mockImplementation((id: string) =>
      pendingWatch.promise.then(() => void backendWatchers.add(id)),
    );
    mocks.unwatchGitCheckout.mockImplementation(async (id: string) => {
      backendWatchers.delete(id);
    });

    const wrapper = mount(host(activeCheckout, activeRepo));
    await flushPromises();
    wrapper.unmount();
    pendingWatch.resolve(undefined);
    await flushPromises();

    expect(mocks.unwatchGitCheckout).toHaveBeenCalledWith("A");
    expect([...backendWatchers]).toEqual([]);
  });

  it("requests default-branch recovery once per checkout configuration", async () => {
    const activeCheckout = ref<Checkout | null>(checkout("first"));
    const activeRepo = ref<Repo | null>(repo());
    const onDefaultBranchUnknown = vi.fn();
    mocks.getGitStatus.mockRejectedValue({ code: "default_branch_unknown", message: "Choose a default branch" });
    const wrapper = mount(host(activeCheckout, activeRepo, onDefaultBranchUnknown));
    await flushPromises();
    expect(onDefaultBranchUnknown).toHaveBeenCalledTimes(1);

    mocks.handlers.get("git-status-changed")?.({ payload: "first" });
    await flushPromises();
    expect(onDefaultBranchUnknown).toHaveBeenCalledTimes(1);

    activeRepo.value = repo("main");
    await flushPromises();
    expect(onDefaultBranchUnknown).toHaveBeenCalledTimes(2);
    expect(mocks.watchGitCheckout).toHaveBeenCalledTimes(1);
    expect(mocks.unwatchGitCheckout).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
