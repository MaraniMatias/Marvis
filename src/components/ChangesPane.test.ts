// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitDiff: vi.fn(),
  getGitDiffPage: vi.fn(),
  getGitStatus: vi.fn(),
  getGitViewedFiles: vi.fn(),
  markGitFileViewed: vi.fn(),
  listen: vi.fn(),
  onStatusChanged: null as ((event: { payload: string }) => void) | null,
  onViewedChanged: null as ((event: { payload: string }) => void) | null,
}));

vi.mock("../lib/ipc", () => ({
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
  getGitStatus: mocks.getGitStatus,
  getGitViewedFiles: mocks.getGitViewedFiles,
  markGitFileViewed: mocks.markGitFileViewed,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

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

describe("ChangesPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.onStatusChanged = null;
    mocks.onViewedChanged = null;
    mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: string }) => void) => {
      if (name === "git-viewed-changed") mocks.onViewedChanged = handler;
      else mocks.onStatusChanged = handler;
      return vi.fn();
    });
    mocks.getGitStatus.mockResolvedValue({
      branch: "feature",
      defaultBranch: "trunk",
      aheadCount: 1,
      files: [
        { path: "text.txt", status: "M" },
        { path: "image.bin", status: "A" },
      ],
    });
    mocks.getGitViewedFiles.mockResolvedValue([]);
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

  it("loads file diffs on demand and refreshes status and the selected diff from watcher events", async () => {
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), defaultBranch: "trunk" },
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

    mocks.onStatusChanged?.({ payload: "checkout:repo" });
    await flushPromises();
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(2);

    await wrapper.get("button:nth-of-type(2)").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Binary file; text diff is unavailable.");

    wrapper.unmount();
  });

  it("delegates an unknown default branch to the existing choice flow", async () => {
    mocks.getGitStatus.mockRejectedValueOnce({
      code: "default_branch_unknown",
      message: "Choose a default branch",
    });
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout() },
    });

    await flushPromises();

    expect(wrapper.emitted("defaultBranchUnknown")).toHaveLength(1);
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
      props: { checkout: checkout(), defaultBranch: "trunk" },
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
      props: { checkout: checkout(), defaultBranch: "trunk" },
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
});
