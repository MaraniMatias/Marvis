// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick, ref } from "vue";
import type { Ref } from "vue";
import type { GitChangedFile } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitCheckoutDiffStats: vi.fn(),
  getGitDiffStats: vi.fn(),
  handlers: new Map<string, (event: { payload: string[] }) => void>(),
  unlisten: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: string[] }) => void) => {
    mocks.handlers.set(name, handler);
    return mocks.unlisten;
  }),
}));

import { useDiffStats } from "./diff-stats";

function checkout(id: string, isMissing = false): Checkout {
  return {
    id,
    repoId: "repo:test",
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing,
    sessions: [],
  };
}

function repo(checkouts: Checkout[]): Repo {
  return {
    id: "repo:test",
    kind: "git",
    name: "test",
    root: "/test",
    defaultBranch: "trunk",
    checkouts,
    createdAt: "now",
    lastOpenedAt: "now",
  };
}

function countedFile(path: string, additions?: number, deletions?: number): GitChangedFile {
  return { path, status: "M", additions, deletions };
}

function host(repos: Ref<Repo[]>, activeId: Ref<string | null>) {
  return defineComponent({
    setup() {
      const stats = useDiffStats(repos, activeId);
      return () =>
        h(
          "div",
          [
            stats.checkoutTotals["checkout:a"] ? `a:${stats.checkoutTotals["checkout:a"].additions}` : "a:none",
            stats.checkoutTotals["checkout:b"] ? `b:${stats.checkoutTotals["checkout:b"].additions}` : "b:none",
            `f:${stats.fileCounts["src/file.ts"]?.additions ?? "none"}`,
          ].join("|"),
        );
    },
  });
}

describe("useDiffStats", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.handlers.clear();
    mocks.getGitCheckoutDiffStats.mockResolvedValue({
      "checkout:a": { additions: 5, deletions: 2 },
      "checkout:b": { additions: 0, deletions: 3 },
    });
    mocks.getGitDiffStats.mockResolvedValue([countedFile("src/file.ts", 5, 2)]);
  });

  /** A change waits out the window before it costs a round trip, unlike a row that just
   *  appeared, which is showing numbers it does not have yet. */
  async function afterDebounce() {
    await vi.advanceTimersByTimeAsync(200);
    await flushPromises();
  }

  it("covers every checkout in one call and asks per file only for the active one", async () => {
    const repos = ref([repo([checkout("checkout:a"), checkout("checkout:b")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();

    // One call for the whole sidebar, not one per row.
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);
    expect(mocks.getGitDiffStats).toHaveBeenCalledTimes(1);
    expect(mocks.getGitDiffStats).toHaveBeenCalledWith("checkout:a");
    expect(wrapper.text()).toBe("a:5|b:0|f:5");

    wrapper.unmount();
  });

  it("refreshes on the same git-status-changed signal that refreshes the file list", async () => {
    vi.useFakeTimers();
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);

    mocks.getGitCheckoutDiffStats.mockResolvedValue({ "checkout:a": { additions: 9, deletions: 9 } });
    // One commit in a worktree moves the merge base every sibling diffs against, so the
    // signal names all of them rather than the one that moved.
    mocks.handlers.get("git-status-changed")?.({ payload: ["checkout:a", "checkout:b"] });
    await afterDebounce();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiffStats).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("a:9");

    wrapper.unmount();
    vi.useRealTimers();
  });

  it("collapses a burst of changes into one call, the way the file list debounces its own", async () => {
    vi.useFakeTimers();
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();

    const changed = mocks.handlers.get("git-status-changed");
    changed?.({ payload: ["checkout:a"] });
    changed?.({ payload: ["checkout:a"] });
    changed?.({ payload: ["checkout:a"] });
    await afterDebounce();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    wrapper.unmount();
    vi.useRealTimers();
  });

  it("does not let a sweep that started for the previous checkout paint the new one's rows", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const activeId = ref<string | null>("checkout:a");
    let releaseFirst!: (files: GitChangedFile[]) => void;
    mocks.getGitDiffStats
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            releaseFirst = resolve;
          }),
      )
      .mockResolvedValue([countedFile("src/file.ts", 40, 41)]);
    const wrapper = mount(host(repos, activeId));
    await flushPromises();

    activeId.value = "checkout:b";
    await nextTick();
    // The sweep already out there belongs to a selection that is gone. Asking again while it
    // runs is what would put two sweeps of the whole workspace side by side.
    releaseFirst([countedFile("src/file.ts", 99, 99)]);
    await flushPromises();

    expect(mocks.getGitDiffStats).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiffStats).toHaveBeenLastCalledWith("checkout:b");
    expect(wrapper.text()).toContain("f:40");
    wrapper.unmount();
  });

  it("asks again for the new checkout when one arrives mid-sweep", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const activeId = ref<string | null>("checkout:a");
    let releaseFirst!: (files: GitChangedFile[]) => void;
    mocks.getGitDiffStats
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            releaseFirst = resolve;
          }),
      )
      .mockResolvedValue([countedFile("src/file.ts", 40, 41)]);
    const wrapper = mount(host(repos, activeId));
    await flushPromises();

    activeId.value = "checkout:b";
    repos.value = [repo([checkout("checkout:a"), checkout("checkout:b")])];
    releaseFirst([countedFile("src/file.ts", 99, 99)]);
    await flushPromises();

    // One answer behind the other, never both at once: the second asks about the checkout
    // that is showing now.
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiffStats).toHaveBeenLastCalledWith("checkout:b");
    expect(wrapper.text()).toContain("f:40");
    wrapper.unmount();
  });

  it("never shows one checkout's file counts on another checkout's rows", async () => {
    const repos = ref([repo([checkout("checkout:a"), checkout("checkout:b")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();
    expect(wrapper.text()).toContain("f:5");

    let release!: (files: GitChangedFile[]) => void;
    mocks.getGitDiffStats.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    activeId.value = "checkout:b";
    await nextTick();
    // Before the new checkout answers, its rows show no number at all: the previous
    // checkout counted a different change set, and the two can share a path.
    expect(wrapper.text()).toContain("f:none");

    release([countedFile("src/file.ts", 40, 41)]);
    await flushPromises();

    expect(mocks.getGitDiffStats).toHaveBeenLastCalledWith("checkout:b");
    expect(wrapper.text()).toContain("f:40");
    wrapper.unmount();
  });

  it("keeps a file Git cannot count without inventing a zero", async () => {
    mocks.getGitDiffStats.mockResolvedValue([countedFile("assets/logo.png")]);
    const repos = ref([repo([checkout("checkout:a")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();

    expect(wrapper.text()).toContain("f:none");
    wrapper.unmount();
  });

  it("keeps the sidebar totals when the per-checkout call fails", async () => {
    mocks.getGitDiffStats.mockRejectedValue(new Error("git failed"));
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();

    expect(wrapper.text()).toContain("a:5");
    wrapper.unmount();
  });

  it("asks again when a checkout joins or leaves the workspace", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);

    repos.value = [repo([checkout("checkout:a"), checkout("checkout:b")])];
    await flushPromises();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("does not ask again when only a session or a branch changed", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();

    repos.value = [repo([{ ...checkout("checkout:a"), sessions: [] }])];
    await flushPromises();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("releases the listener and the numbers when the last reader unmounts", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();

    wrapper.unmount();
    await flushPromises();

    expect(mocks.unlisten).toHaveBeenCalledTimes(1);
  });
});
