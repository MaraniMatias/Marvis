// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
import type { GitDiffPage } from "../domain/git";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";

const mocks = vi.hoisted(() => ({
  getGitDiff: vi.fn(),
  getGitDiffPage: vi.fn(),
  markGitFileViewed: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
  markGitFileViewed: mocks.markGitFileViewed,
}));

vi.mock("@git-diff-view/vue", () => ({
  DiffFile: class {
    initTheme() {}
    init() {}
    buildUnifiedDiffLines() {}
  },
  DiffModeEnum: { Unified: 4 },
  DiffView: { template: "<div />" },
}));

import ChangesPane from "./ChangesPane.vue";

function checkout(): Checkout {
  return {
    id: "checkout:repo",
    repoId: "repo:repo",
    path: "/repo",
    canonicalPath: "/repo",
    isPrimary: true,
    changedFiles: 0,
    isMissing: false,
    sessions: [],
  };
}

function gitSnapshot(
  status: ActiveGitSnapshot["status"] = {
    branch: "feature",
    defaultBranch: "trunk",
    aheadCount: 1,
    files: [
      { path: "text.txt", status: "M" },
      { path: "image.bin", status: "A" },
    ],
  },
  overrides: Partial<ActiveGitSnapshot> = {},
): ActiveGitSnapshot {
  const snapshot = reactive<ActiveGitSnapshot>({
    checkoutId: "checkout:repo",
    status,
    viewedPaths: [],
    loading: false,
    statusState: status ? "ready" : "error",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: async (checkoutId, path) => {
      try {
        await mocks.markGitFileViewed(checkoutId, path);
        if (snapshot.checkoutId === checkoutId && !snapshot.viewedPaths.includes(path)) {
          snapshot.viewedPaths = [...snapshot.viewedPaths, path];
        }
      } catch (error) {
        snapshot.viewedError = error instanceof Error ? error.message : String(error);
      }
    },
    ...overrides,
  });
  return snapshot;
}

function largeDiff(totalLines: number) {
  return {
    path: "text.txt",
    patch: "",
    isBinary: false,
    large: true,
    tooLarge: false,
    totalLines,
    hunks: [{ startLine: 0, endLine: totalLines, title: "@@ -1 +1 @@" }],
  };
}

function diffPage(totalLines: number, offset: number, limit: number): GitDiffPage {
  return {
    path: "text.txt",
    startLine: offset,
    totalLines,
    lines: Array.from({ length: Math.min(limit, totalLines - offset) }, (_, pageIndex) => {
      const index = offset + pageIndex;
      return {
        index,
        kind: index === 0 ? "hunk" : "context",
        text: index === 0 ? "@@ -1 +1 @@" : ` line-${index}`,
        oldLineNumber: index || null,
        newLineNumber: index || null,
      };
    }),
  };
}

describe("ChangesPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.markGitFileViewed.mockResolvedValue(undefined);
    mocks.getGitDiffPage.mockResolvedValue({ path: "", startLine: 0, totalLines: 0, lines: [] });
    mocks.getGitDiff.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      patch:
        path === "text.txt"
          ? "diff --git a/text.txt b/text.txt\n--- a/text.txt\n+++ b/text.txt\n@@ -1 +1 @@\n-old\n+new\n"
          : "",
      isBinary: path === "image.bin",
      large: false,
      tooLarge: false,
      totalLines: path === "text.txt" ? 3 : 0,
      hunks: path === "text.txt" ? [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }] : [],
    }));
  });

  it("uses the shared status and refreshes a selected diff when its revision changes", async () => {
    const snapshot = gitSnapshot();
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: snapshot },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("vs trunk · 2 changed");
    expect(mocks.getGitDiff).not.toHaveBeenCalled();

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(1);
    expect(mocks.markGitFileViewed).toHaveBeenCalledWith("checkout:repo", "text.txt");
    expect(wrapper.text()).toContain("1/2 viewed");
    const hunk = wrapper.get('[aria-expanded="true"]');
    await hunk.trigger("click");
    expect(hunk.attributes("aria-expanded")).toBe("false");

    snapshot.statusRevision += 1;
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(2);

    await wrapper.get("button:nth-of-type(2)").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Binary file; text diff is unavailable.");

    wrapper.unmount();
  });

  it("shows status errors owned by the active checkout snapshot", async () => {
    const wrapper = mount(ChangesPane, {
      props: {
        checkout: checkout(),
        gitSnapshot: gitSnapshot(null, {
          statusState: "error",
          changesStatusError: "Choose a default branch",
        }),
      },
    });

    await flushPromises();

    expect(wrapper.text()).toContain("Choose a default branch");
    wrapper.unmount();
  });

  it("shows an explicit limit warning and viewed progress for a diff beyond safety limits", async () => {
    mocks.getGitDiff.mockResolvedValueOnce({
      path: "text.txt",
      patch: "",
      isBinary: false,
      large: true,
      tooLarge: true,
      totalLines: 100001,
      hunks: [],
    });
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("This diff exceeds safe preview limits");
    expect(wrapper.text()).toContain("0/2 viewed");
    wrapper.unmount();
  });

  it("loads the last rows of a 5,000-line diff while keeping the rendered row window bounded", async () => {
    const totalLines = 10001;
    mocks.getGitDiff.mockResolvedValueOnce({
      path: "text.txt",
      patch: "",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines,
      hunks: [{ startLine: 0, endLine: totalLines, title: "@@ -1,5000 +1,5000 @@" }],
    });
    mocks.getGitDiffPage.mockImplementation(
      async (_checkoutId: string, path: string, offset: number, limit: number) => ({
        path,
        startLine: offset,
        totalLines,
        lines: Array.from({ length: Math.min(limit, totalLines - offset) }, (_, pageIndex) => {
          const index = offset + pageIndex;
          if (index === 0) return { index, kind: "hunk", text: "@@ -1,5000 +1,5000 @@" };
          if (index <= 5000) {
            return { index, kind: "removed", text: `-old-${index - 1}`, oldLineNumber: index };
          }
          return { index, kind: "added", text: `+new-${index - 5001}`, newLineNumber: index - 5000 };
        }),
      }),
    );

    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();

    const viewport = wrapper.get('[aria-label="Large diff"]');
    (viewport.element as HTMLElement).scrollTop = totalLines * 22;
    await viewport.trigger("scroll");
    await flushPromises();

    expect(wrapper.text()).toContain("+new-4999");
    expect(wrapper.findAll('[data-testid="large-diff-row"]').length).toBeLessThanOrEqual(80);
    expect(mocks.getGitDiffPage).toHaveBeenCalledWith("checkout:repo", "text.txt", 9984, 32);
    wrapper.unmount();
  });

  it("keeps paged diff requests to three while the window spans four pages", async () => {
    const totalLines = 500;
    mocks.getGitDiff.mockResolvedValueOnce(largeDiff(totalLines));
    let active = 0;
    let maximumActive = 0;
    const resolvePage = new Map<number, () => void>();
    mocks.getGitDiffPage.mockImplementation(
      (_checkoutId: string, _path: string, offset: number, limit: number) =>
        new Promise<GitDiffPage>((resolve) => {
          active += 1;
          maximumActive = Math.max(maximumActive, active);
          resolvePage.set(offset, () => {
            resolvePage.delete(offset);
            active -= 1;
            resolve(diffPage(totalLines, offset, limit));
          });
        }),
    );

    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();
    const viewport = wrapper.get('[aria-label="Large diff"]');
    (viewport.element as HTMLElement).scrollTop = 27 * 22;
    await viewport.trigger("scroll");
    await flushPromises();

    expect(mocks.getGitDiffPage).toHaveBeenCalledTimes(3);
    expect(maximumActive).toBe(3);
    resolvePage.get(0)?.();
    await flushPromises();
    expect(mocks.getGitDiffPage).toHaveBeenCalledWith("checkout:repo", "text.txt", 96, 32);
    expect(maximumActive).toBe(3);

    for (const resolve of [...resolvePage.values()]) resolve();
    await flushPromises();
    wrapper.unmount();
  });

  it("evicts old diff pages after eight cached pages", async () => {
    const totalLines = 1024;
    mocks.getGitDiff.mockResolvedValueOnce(largeDiff(totalLines));
    mocks.getGitDiffPage.mockImplementation(async (_checkoutId: string, _path: string, offset: number, limit: number) =>
      diffPage(totalLines, offset, limit),
    );

    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();
    const viewport = wrapper.get('[aria-label="Large diff"]');
    const scrollToWindow = async (start: number) => {
      (viewport.element as HTMLElement).scrollTop = (start + 10) * 22;
      await viewport.trigger("scroll");
      await flushPromises();
    };
    await scrollToWindow(96);
    await scrollToWindow(192);
    await scrollToWindow(0);

    const requestedOffsets = mocks.getGitDiffPage.mock.calls.map(([, , offset]) => offset);
    expect(new Set(requestedOffsets.slice(0, 9)).size).toBe(9);
    expect(requestedOffsets.filter((offset) => offset === 0)).toHaveLength(2);
    wrapper.unmount();
  });

  it("ignores page responses that finish after selecting another file", async () => {
    const totalLines = 100;
    mocks.getGitDiff.mockImplementation(async (_checkoutId: string, path: string) =>
      path === "text.txt" ? largeDiff(totalLines) : { ...largeDiff(0), isBinary: true, large: false },
    );
    const resolvePages: Array<() => void> = [];
    mocks.getGitDiffPage.mockImplementation(
      (_checkoutId: string, _path: string, offset: number, limit: number) =>
        new Promise<GitDiffPage>((resolve) => {
          resolvePages.push(() => resolve(diffPage(totalLines, offset, limit)));
        }),
    );

    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(resolvePages.length).toBeGreaterThan(0);

    await wrapper.get("button:nth-of-type(2)").trigger("click");
    await flushPromises();
    for (const resolve of resolvePages) resolve();
    await flushPromises();

    expect(wrapper.text()).toContain("Binary file; text diff is unavailable.");
    expect(wrapper.find('[data-testid="large-diff-row"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("invalidates pending page responses when the checkout changes", async () => {
    const totalLines = 100;
    mocks.getGitDiff.mockResolvedValueOnce(largeDiff(totalLines));
    const resolvePages: Array<() => void> = [];
    mocks.getGitDiffPage.mockImplementation(
      (_checkoutId: string, _path: string, offset: number, limit: number) =>
        new Promise<GitDiffPage>((resolve) => {
          resolvePages.push(() => resolve(diffPage(totalLines, offset, limit)));
        }),
    );

    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), gitSnapshot: gitSnapshot() },
    });
    await flushPromises();
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(resolvePages.length).toBeGreaterThan(0);

    const nextSnapshot = gitSnapshot();
    nextSnapshot.checkoutId = "checkout:other";
    await wrapper.setProps({
      checkout: { ...checkout(), id: "checkout:other" },
      gitSnapshot: nextSnapshot,
    });
    for (const resolve of resolvePages) resolve();
    await flushPromises();

    expect(wrapper.text()).toContain("Select a changed file to load its diff.");
    expect(wrapper.find('[data-testid="large-diff-row"]').exists()).toBe(false);
    wrapper.unmount();
  });
});
