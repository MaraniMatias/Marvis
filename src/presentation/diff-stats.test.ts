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
  handlers: new Map<string, (event: { payload: string }) => void>(),
  unlisten: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: string }) => void) => {
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
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);

    mocks.getGitCheckoutDiffStats.mockResolvedValue({ "checkout:a": { additions: 9, deletions: 9 } });
    mocks.handlers.get("git-status-changed")?.({ payload: "checkout:a" });
    await flushPromises();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiffStats).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("a:9");

    wrapper.unmount();
  });

  it("collapses a burst of changes into one call, the way the file list debounces its own", async () => {
    const repos = ref([repo([checkout("checkout:a")])]);
    const wrapper = mount(host(repos, ref("checkout:a")));
    await flushPromises();

    const changed = mocks.handlers.get("git-status-changed");
    changed?.({ payload: "checkout:a" });
    changed?.({ payload: "checkout:a" });
    changed?.({ payload: "checkout:a" });
    await flushPromises();

    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("never shows one checkout's file counts on another checkout's rows", async () => {
    const repos = ref([repo([checkout("checkout:a"), checkout("checkout:b")])]);
    const activeId = ref<string | null>("checkout:a");
    const wrapper = mount(host(repos, activeId));
    await flushPromises();
    expect(wrapper.text()).toContain("f:5");

    let release!: () => void;
    mocks.getGitDiffStats.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () => resolve([countedFile("src/file.ts", 40, 41)]);
        }),
    );
    activeId.value = "checkout:b";
    await nextTick();
    // Before the new checkout answers, its rows show no number at all: the previous
    // checkout counted a different change set, and the two can share a path.
    expect(wrapper.text()).toContain("f:none");

    release();
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
